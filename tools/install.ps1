# Install or update a signed Gent release on Windows. Run only from a trusted source.
[CmdletBinding()]
param(
    [string]$Version = $env:GENT_VERSION,
    [string]$InstallDir = $env:GENT_INSTALL_DIR,
    [string]$ExpectedSha256,
    [string]$IdleDataDir,
    [switch]$Force,
    [switch]$RequireHealth
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$Repository = "gent-ar/gent-cli"
$Target = "x86_64-pc-windows-msvc"
$Identity = "^https://github.com/$Repository/.github/workflows/release.yml@refs/tags/"
function Fail([string]$Message) {
    throw "Gent install failed: $Message"
}
function Require-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        Fail "$Name is required for signed Gent installs"
    }
}
function Get-ReleaseVersion {
    if ($Version) { return $Version }
    $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repository/releases/latest"
    return [string]$latest.tag_name
}
function Assert-Version([string]$Candidate) {
    if (-not [regex]::IsMatch($Candidate, '\Av[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?\z')) {
        Fail "invalid release version: $Candidate"
    }
}
function Assert-ExpectedDigest([string]$Digest) {
    if ($Digest -and $Digest -notmatch '^[0-9a-f]{64}$') {
        Fail "expected digest must be 64 lowercase hexadecimal characters"
    }
}
function Get-AssetBase([string]$ReleaseVersion) {
    if ($env:GENT_RELEASE_BASE_URL) {
        return $env:GENT_RELEASE_BASE_URL.TrimEnd('/')
    }
    return "https://github.com/$Repository/releases/download/$ReleaseVersion"
}
function Get-Asset([string]$Base, [string]$Name, [string]$Directory) {
    $destination = Join-Path $Directory $Name
    Invoke-WebRequest -Uri "$Base/$Name" -OutFile $destination
    return $destination
}
function Assert-Signed([string]$Path, [string]$Bundle, [string]$ReleaseVersion) {
    $tag = [regex]::Escape($ReleaseVersion)
    & cosign verify-blob $Path --bundle $Bundle `
        --certificate-identity-regexp "$Identity$tag`$" `
        --certificate-oidc-issuer "https://token.actions.githubusercontent.com" | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail "signature verification failed for $(Split-Path $Path -Leaf)" }
}
function Assert-ReleaseName([string]$Name) {
    if ($Name -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?-x86_64-pc-windows-msvc$') {
        Fail "invalid release identity"
    }
}
function Assert-PlainFile([string]$Path, [string]$Description) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { Fail "$Description is not a file" }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Fail "$Description cannot be a reparse point"
    }
}
function Assert-PlainDirectory([string]$Path, [string]$Description) {
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { Fail "$Description is not a directory" }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Fail "$Description cannot be a reparse point"
    }
}
function Remove-PrivateDirectory([string]$Path, [string]$Parent, [string]$Prefix) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    Assert-PlainDirectory $Path "private staging directory"
    $actual = [IO.Path]::GetFullPath($Path)
    $expectedParent = [IO.Path]::GetFullPath($Parent).TrimEnd('\')
    if ([IO.Path]::GetDirectoryName($actual).TrimEnd('\') -ne $expectedParent -or -not [IO.Path]::GetFileName($actual).StartsWith($Prefix, [StringComparison]::Ordinal)) { Fail "private directory cleanup escaped its expected parent" }
    Remove-Item -LiteralPath $actual -Force -Recurse
}
function Move-Atomically([string]$Source, [string]$Destination) {
    if (-not ("GentAtomicMove" -as [type])) { Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class GentAtomicMove {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern bool MoveFileEx(string source, string destination, int flags);
    public static void Replace(string source, string destination) {
        if (!MoveFileEx(source, destination, 1 | 8)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
}
'@
    }
    [GentAtomicMove]::Replace($Source, $Destination)
}
function Write-CurrentPointer([string]$RuntimeRoot, [string]$ReleaseName) {
    Assert-MutationLease
    Assert-ReleaseName $ReleaseName
    $releasePath = Join-Path (Join-Path $RuntimeRoot "releases") $ReleaseName
    Assert-PlainDirectory $releasePath "selected release"
    foreach ($binary in @("gent.exe", "gentd.exe")) {
        $path = Join-Path $releasePath $binary
        Assert-PlainFile $path "selected release $binary"
    }
    Assert-PackagedRuntime $releasePath
    $pointer = Join-Path $RuntimeRoot "current.json"
    $temporary = Join-Path $RuntimeRoot (".current-" + [Guid]::NewGuid().ToString("N"))
    $json = @{ release = $ReleaseName } | ConvertTo-Json -Compress
    [System.IO.File]::WriteAllText($temporary, $json, [System.Text.UTF8Encoding]::new($false))
    if (Test-Path -LiteralPath $pointer -PathType Container) { Fail "current pointer is not a file" }
    if (Test-Path -LiteralPath $pointer) {
        Assert-PlainFile $pointer "current pointer"
        Move-Atomically $temporary $pointer
    }
    else { [System.IO.File]::Move($temporary, $pointer) }
}
function Write-NativeLaunchers([string]$Root, [string]$ReleasePath) {
    Assert-MutationLease
    $bin = Join-Path $Root "bin"
    New-Item -ItemType Directory -Force -Path $bin | Out-Null
    $source = Join-Path $ReleasePath "gent-launcher.exe"
    Assert-PlainFile $source "release gent-launcher.exe"
    $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash
    foreach ($binary in @("gent", "gentd")) {
        $destination = Join-Path $bin "$binary.exe"
        if (Test-Path -LiteralPath $destination -PathType Container) { Fail "$destination is not a file" }
        if (Test-Path -LiteralPath $destination) { Assert-PlainFile $destination "existing native launcher" }
        $temporary = Join-Path $bin (".$binary-" + [Guid]::NewGuid().ToString("N") + ".exe")
        [IO.File]::Copy($source, $temporary)
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath $temporary).Hash -ne $sourceHash) {
            Remove-Item -LiteralPath $temporary -Force
            Fail "native launcher copy verification failed"
        }
        if (Test-Path -LiteralPath $destination) { Move-Atomically $temporary $destination }
        else { [IO.File]::Move($temporary, $destination) }
    }
}
function Test-StagedPair([string]$ReleasePath) {
    $data = Join-Path ([IO.Path]::GetTempPath()) ("gent-health-" + [Guid]::NewGuid().ToString("N")); $process = $null
    try {
        $process = Start-Process -FilePath (Join-Path $ReleasePath "gentd.exe") -ArgumentList @("--data-dir", $data) -PassThru -WindowStyle Hidden
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        while ([DateTime]::UtcNow -lt $deadline) {
            & (Join-Path $ReleasePath "gent.exe") --data-dir $data --no-autostart status 2>$null | Out-Null
            if ($LASTEXITCODE -eq 0) { return }
            if ($process.HasExited) { Fail "staged gentd exited before its local IPC health check" }
            Start-Sleep -Milliseconds 100
        }
        Fail "staged runtime did not complete a local IPC health check"
    } finally { if ($null -ne $process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force }; Remove-PrivateDirectory $data ([IO.Path]::GetTempPath()) "gent-health-" }
}
function Write-AutoUpdater([string]$Source, [string]$Root) {
    Assert-MutationLease
    $destination = Join-Path $Root "gent-auto-update.ps1"; $temporary = Join-Path $Root (".gent-auto-update-" + [Guid]::NewGuid().ToString("N"))
    [IO.File]::Copy($Source, $temporary); if (Test-Path -LiteralPath $destination) { Assert-PlainFile $destination "automatic-update helper"; Move-Atomically $temporary $destination } else { [IO.File]::Move($temporary, $destination) }
}
function Enable-AutoUpdater([string]$Root, [string]$DataDir) {
    $marker = Join-Path $Root ".gent-auto-update-disabled"; if (Test-Path -LiteralPath $marker) { Assert-PlainFile $marker "automatic-update preference marker"; return }
    $data = if ($DataDir) { $DataDir } else { Join-Path $env:LOCALAPPDATA "Gent\Gent\data" }; $arguments = @("enable", "-RuntimeRoot", $Root, "-DataDir", $data)
    if ($env:GENT_AUTO_UPDATE_SCHEDULER_DIR) { $arguments += @("-SchedulerDir", $env:GENT_AUTO_UPDATE_SCHEDULER_DIR) }
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $Root "gent-auto-update.ps1") @arguments
    if ($LASTEXITCODE -ne 0) { Fail "automatic-update scheduler setup failed" }
}
Require-Command "cosign"
$Version = Get-ReleaseVersion
Assert-Version $Version
Assert-ExpectedDigest $ExpectedSha256
if (-not $InstallDir) {
    if (-not $env:LOCALAPPDATA) { Fail "LOCALAPPDATA is unavailable; pass --InstallDir" }
    $InstallDir = Join-Path $env:LOCALAPPDATA "Gent"
}
$archiveName = "gent-$Version-$Target.zip"
$base = Get-AssetBase $Version
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ("gent-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $temporary | Out-Null
$privateStage = $null
try {
    $archive = Get-Asset $base $archiveName $temporary
    $checksum = Get-Asset $base "$archiveName.sha256" $temporary
    $manifest = Get-Asset $base "$archiveName.manifest.json" $temporary
    $archiveBundle = Get-Asset $base "$archiveName.sigstore.json" $temporary
    $manifestBundle = Get-Asset $base "$archiveName.manifest.json.sigstore.json" $temporary
    $autoUpdater = Get-Asset $base "gent-auto-update.ps1" $temporary
    $autoUpdaterBundle = Get-Asset $base "gent-auto-update.ps1.sigstore.json" $temporary
    $support = Get-Asset $base "gent-install-support.ps1" $temporary
    $supportBundle = Get-Asset $base "gent-install-support.ps1.sigstore.json" $temporary
    Assert-Signed $archive $archiveBundle $Version
    Assert-Signed $manifest $manifestBundle $Version
    Assert-Signed $autoUpdater $autoUpdaterBundle $Version
    Assert-Signed $support $supportBundle $Version
    . $support
    Assert-Archive $archive $manifest $checksum $Version
    $actualDigest = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    if ($ExpectedSha256 -and $ExpectedSha256 -ne $actualDigest) {
        Fail "release digest does not match explicit update confirmation"
    }
    Assert-ZipMembers $archive $Version
    $current = Assert-NoNewerSelection $InstallDir $Version
    if ($current -and -not $IdleDataDir) { Fail "updating an existing runtime requires explicit -IdleDataDir" }
    $extracted = Join-Path $temporary "extracted"
    [IO.Compression.ZipFile]::ExtractToDirectory($archive, $extracted)
    $source = Join-Path $extracted "gent-$Version-$Target"
    Assert-PackagedRuntime $source
    $sourceDigest = Get-RuntimeTreeDigest $source
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Assert-PlainDirectory $InstallDir "runtime root"
    $privateStage = Join-Path $InstallDir (".gent-stage-" + [Guid]::NewGuid().ToString("N"))
    [IO.Directory]::CreateDirectory($privateStage) | Out-Null
    Get-ChildItem -LiteralPath $source -Force | Copy-Item -Destination $privateStage -Recurse
    if ((Get-RuntimeTreeDigest $source) -ne $sourceDigest -or (Get-RuntimeTreeDigest $privateStage) -ne $sourceDigest) { Fail "private runtime copy changed identity" }
    $stagedUpdater = Join-Path $privateStage "gent-auto-update.ps1"
    Copy-Item -LiteralPath $autoUpdater -Destination $stagedUpdater
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $stagedUpdater).Hash -ne (Get-FileHash -Algorithm SHA256 -LiteralPath $autoUpdater).Hash) { Fail "private updater copy changed identity" }
    $sourceDigest = Get-RuntimeTreeDigest $privateStage
    if ($RequireHealth) { Test-StagedPair $privateStage }
    Use-IdleDaemonLock $IdleDataDir {
        Use-InstallLock $InstallDir {
            $previous = Assert-NoNewerSelection $InstallDir $Version
            $previousDigest = $null
            if ($previous) { try { $previousDigest = Get-RuntimeTreeDigest (Join-Path (Join-Path $InstallDir "releases") $previous) } catch { } }
            if ($previous -and -not $Force) { Fail "Gent is already installed; pass -Force to update it" }
            $releaseName = Publish-ImmutableRelease $InstallDir $privateStage $sourceDigest $Version
            $release = Join-Path (Join-Path $InstallDir "releases") $releaseName
            Write-NativeLaunchers $InstallDir $release
            Write-AutoUpdater $autoUpdater $InstallDir
            try {
                Write-CurrentPointer $InstallDir $releaseName
                if ((Read-SelectedRelease $InstallDir) -ne $releaseName -or (Get-RuntimeTreeDigest $release) -ne $sourceDigest) { Fail "selected runtime identity could not be verified" }
            } catch {
                if ((Read-SelectedRelease $InstallDir) -eq $releaseName -and $previous -ne $releaseName) { Restore-ExactSelection $InstallDir $previous $previousDigest $releaseName }
                throw
            }
        }
    }
    Enable-AutoUpdater $InstallDir $IdleDataDir
    Write-Output "Installed Gent $Version in $(Join-Path $InstallDir 'bin')"
    Write-Output "Add $(Join-Path $InstallDir 'bin') to PATH, then run: gent doctor"
}
finally {
    if ($privateStage) { Remove-PrivateDirectory $privateStage $InstallDir ".gent-stage-" }
    Remove-PrivateDirectory $temporary ([IO.Path]::GetTempPath()) "gent-install-"
}
