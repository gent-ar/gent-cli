# Install or update a signed Gent release on Windows. Run only from a trusted source.
[CmdletBinding()]
param(
    [string]$Version = $env:GENT_VERSION,
    [string]$InstallDir = $env:GENT_INSTALL_DIR,
    [string]$ExpectedSha256,
    [string]$IdleDataDir,
    [switch]$Force
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
        --certificate-oidc-issuer "https://github.com/login/oauth" | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail "signature verification failed for $(Split-Path $Path -Leaf)" }
}

function Assert-Archive([string]$Archive, [string]$ManifestPath, [string]$ChecksumPath,
    [string]$ReleaseVersion) {
    $manifest = Get-Content -Raw -LiteralPath $ManifestPath | ConvertFrom-Json
    $digest = (Get-FileHash -Algorithm SHA256 -LiteralPath $Archive).Hash.ToLowerInvariant()
    $expectedName = Split-Path $Archive -Leaf
    $expectedChecksum = "$digest  $expectedName"
    if ($manifest.schemaVersion -ne 1 -or $manifest.version -ne $ReleaseVersion -or
        $manifest.target -ne $Target -or $manifest.archive.name -ne $expectedName -or
        $manifest.archive.sha256 -ne $digest -or $manifest.archive.size -ne (Get-Item -LiteralPath $Archive).Length -or
        (Get-Content -Raw -LiteralPath $ChecksumPath).Trim() -ne $expectedChecksum) {
        Fail "release archive verification failed"
    }
    $binaries = @($manifest.binaries | Sort-Object)
    if (($binaries -join ',') -ne 'gent.exe,gentd.exe,gent-launcher.exe') { Fail "release manifest has invalid binaries" }
}

function Assert-ZipMembers([string]$Archive, [string]$ReleaseVersion) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $root = "gent-$ReleaseVersion-$Target"
    $expected = @("$root/gent.exe", "$root/gentd.exe", "$root/gent-launcher.exe")
    $zip = [System.IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $members = @($zip.Entries | ForEach-Object { $_.FullName } | Sort-Object)
        $expectedMembers = @($expected | Sort-Object)
        if (($members -join ',') -ne ($expectedMembers -join ',')) {
            Fail "release archive contains unsafe or unexpected paths"
        }
        foreach ($entry in $zip.Entries) {
            if ($entry.FullName.Contains('..') -or $entry.FullName.StartsWith('/') -or
                $entry.FullName -match '^[A-Za-z]:' -or $entry.Name.Length -eq 0) {
                Fail "release archive contains an unsafe path"
            }
        }
    }
    finally { $zip.Dispose() }
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
    Assert-ReleaseName $ReleaseName
    $releasePath = Join-Path (Join-Path $RuntimeRoot "releases") $ReleaseName
    Assert-PlainDirectory $releasePath "selected release"
    foreach ($binary in @("gent.exe", "gentd.exe")) {
        $path = Join-Path $releasePath $binary
        Assert-PlainFile $path "selected release $binary"
    }
    $pointer = Join-Path $RuntimeRoot "current.json"
    $temporary = Join-Path $RuntimeRoot (".current-" + [Guid]::NewGuid().ToString("N"))
    @{ release = $ReleaseName } | ConvertTo-Json -Compress | Set-Content -LiteralPath $temporary -NoNewline -Encoding utf8
    if (Test-Path -LiteralPath $pointer -PathType Container) { Fail "current pointer is not a file" }
    if (Test-Path -LiteralPath $pointer) {
        Assert-PlainFile $pointer "current pointer"
        Move-Atomically $temporary $pointer
    }
    else { [System.IO.File]::Move($temporary, $pointer) }
}

function Write-NativeLaunchers([string]$Root, [string]$ReleasePath) {
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
        $legacy = Join-Path $bin "$binary.cmd"
        if (Test-Path -LiteralPath $legacy) {
            Assert-PlainFile $legacy "legacy launcher"
            Remove-Item -LiteralPath $legacy -Force
        }
    }
}

function Assert-IdenticalRelease([string]$Expected, [string]$Installed) {
    Assert-PlainDirectory $Expected "extracted release"
    Assert-PlainDirectory $Installed "existing release"
    foreach ($binary in @("gent.exe", "gentd.exe", "gent-launcher.exe")) {
        $expectedPath = Join-Path $Expected $binary
        $installedPath = Join-Path $Installed $binary
        Assert-PlainFile $expectedPath "extracted release $binary"
        Assert-PlainFile $installedPath "existing release $binary"
        $expectedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $expectedPath).Hash
        $installedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installedPath).Hash
        if ($expectedHash -ne $installedHash) { Fail "existing release does not match signed archive" }
    }
}

function Use-InstallLock([string]$Root, [scriptblock]$Action) {
    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    $path = Join-Path $Root ".install.lock"
    if (Test-Path -LiteralPath $path) { Assert-PlainFile $path "installer lock" }
    $lock = [System.IO.File]::Open($path, "OpenOrCreate", "ReadWrite", "Read")
    try {
        try { $lock.Lock(0, 1) }
        catch { Fail "another Gent install is already in progress" }
        try { & $Action }
        finally { $lock.Unlock(0, 1) }
    }
    finally { $lock.Dispose() }
}

function Use-IdleDaemonLock([string]$DataDir, [scriptblock]$Action) {
    if (-not $DataDir) { & $Action; return }
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    $path = Join-Path $DataDir "gentd.lock"
    $lock = [System.IO.File]::Open($path, "OpenOrCreate", "ReadWrite", "ReadWrite")
    try {
        try { $lock.Lock(0, 1) }
        catch { Fail "gentd is running for $DataDir; stop it before updating" }
        try { & $Action }
        finally { $lock.Unlock(0, 1) }
    }
    finally { $lock.Dispose() }
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
$releaseName = "$Version-$Target"
$base = Get-AssetBase $Version
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ("gent-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $temporary | Out-Null
try {
    $archive = Get-Asset $base $archiveName $temporary
    $checksum = Get-Asset $base "$archiveName.sha256" $temporary
    $manifest = Get-Asset $base "$archiveName.manifest.json" $temporary
    $archiveBundle = Get-Asset $base "$archiveName.sigstore.json" $temporary
    $manifestBundle = Get-Asset $base "$archiveName.manifest.json.sigstore.json" $temporary
    Assert-Signed $archive $archiveBundle $Version
    Assert-Signed $manifest $manifestBundle $Version
    Assert-Archive $archive $manifest $checksum $Version
    $actualDigest = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    if ($ExpectedSha256 -and $ExpectedSha256 -ne $actualDigest) {
        Fail "release digest does not match explicit update confirmation"
    }
    Assert-ZipMembers $archive $Version
    Use-InstallLock $InstallDir {
        $pointer = Join-Path $InstallDir "current.json"
        if ((Test-Path -LiteralPath $pointer) -and -not $Force) { Fail "Gent is already installed; pass --Force to update it" }
        $releases = Join-Path $InstallDir "releases"
        $release = Join-Path $releases $releaseName
        New-Item -ItemType Directory -Force -Path $releases | Out-Null
        $stage = Join-Path $releases (".stage-" + [Guid]::NewGuid().ToString("N"))
        [System.IO.Compression.ZipFile]::ExtractToDirectory($archive, $stage)
        $source = Join-Path $stage "gent-$Version-$Target"
        if (Test-Path -LiteralPath $release) {
            Assert-IdenticalRelease $source $release
        }
        else { Move-Item -LiteralPath $source -Destination $release }
        Remove-Item -LiteralPath $stage -Force -Recurse
        Write-NativeLaunchers $InstallDir $release
        Use-IdleDaemonLock $IdleDataDir { Write-CurrentPointer $InstallDir $releaseName }
    }
    Write-Output "Installed Gent $Version in $(Join-Path $InstallDir 'bin')"
    Write-Output "Add $(Join-Path $InstallDir 'bin') to PATH, then run: gent doctor"
}
finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force -Recurse }
}
