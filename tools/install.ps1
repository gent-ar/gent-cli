# Install or update a signed Gent release on Windows. Run only from a trusted source.
[CmdletBinding()]
param(
    [string]$Version = $env:GENT_VERSION,
    [string]$InstallDir = $env:GENT_INSTALL_DIR,
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
    if ($Candidate -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$') {
        Fail "invalid release version: $Candidate"
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
    & cosign verify-blob $Path --bundle $Bundle `
        --certificate-identity-regexp "$Identity$ReleaseVersion$" `
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
    if (($binaries -join ',') -ne 'gent.exe,gentd.exe') { Fail "release manifest has invalid binaries" }
}

function Assert-ZipMembers([string]$Archive, [string]$ReleaseVersion) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $root = "gent-$ReleaseVersion-$Target"
    $expected = @("$root/gent.exe", "$root/gentd.exe")
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

function Write-CurrentPointer([string]$RuntimeRoot, [string]$ReleaseName) {
    Assert-ReleaseName $ReleaseName
    $releasePath = Join-Path (Join-Path $RuntimeRoot "releases") $ReleaseName
    foreach ($binary in @("gent.exe", "gentd.exe")) {
        $path = Join-Path $releasePath $binary
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Fail "release is missing $binary" }
    }
    $pointer = Join-Path $RuntimeRoot "current.json"
    $temporary = Join-Path $RuntimeRoot (".current-" + [Guid]::NewGuid().ToString("N"))
    @{ release = $ReleaseName } | ConvertTo-Json -Compress | Set-Content -LiteralPath $temporary -NoNewline -Encoding utf8
    if (Test-Path -LiteralPath $pointer -PathType Container) { Fail "current pointer is not a file" }
    if (Test-Path -LiteralPath $pointer) {
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
        [GentAtomicMove]::Replace($temporary, $pointer)
    }
    else { [System.IO.File]::Move($temporary, $pointer) }
}

function Write-Launchers([string]$Root) {
    $bin = Join-Path $Root "bin"
    New-Item -ItemType Directory -Force -Path $bin | Out-Null
    foreach ($binary in @("gent", "gentd")) {
        $template = @'
@echo off
setlocal
set ROOT=%~dp0..
for /f "usebackq delims=" %%R in (`powershell.exe -NoProfile -NonInteractive -Command "$value=(Get-Content -Raw -LiteralPath '%ROOT%\current.json' | ConvertFrom-Json).release; if ($value -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?-x86_64-pc-windows-msvc$') { exit 1 }; [Console]::Write($value)"`) do set "RELEASE=%%R"
if not exist "%ROOT%\releases\%RELEASE%\__BINARY__.exe" exit /b 1
"%ROOT%\releases\%RELEASE%\__BINARY__.exe" %*
'@
        $content = $template.Replace("__BINARY__", $binary)
        Set-Content -LiteralPath (Join-Path $bin "$binary.cmd") -Value $content -NoNewline -Encoding ascii
    }
}

Require-Command "cosign"
$Version = Get-ReleaseVersion
Assert-Version $Version
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
    Assert-ZipMembers $archive $Version
    $pointer = Join-Path $InstallDir "current.json"
    if ((Test-Path -LiteralPath $pointer) -and -not $Force) { Fail "Gent is already installed; pass --Force to update it" }
    $releases = Join-Path $InstallDir "releases"
    $release = Join-Path $releases $releaseName
    New-Item -ItemType Directory -Force -Path $releases | Out-Null
    if (-not (Test-Path -LiteralPath $release)) {
        $stage = Join-Path $releases (".stage-" + [Guid]::NewGuid().ToString("N"))
        [System.IO.Compression.ZipFile]::ExtractToDirectory($archive, $stage)
        $source = Join-Path $stage "gent-$Version-$Target"
        Move-Item -LiteralPath $source -Destination $release
        Remove-Item -LiteralPath $stage -Force -Recurse
    }
    Write-CurrentPointer $InstallDir $releaseName
    Write-Launchers $InstallDir
    Write-Output "Installed Gent $Version in $(Join-Path $InstallDir 'bin')"
    Write-Output "Add $(Join-Path $InstallDir 'bin') to PATH, then run: gent doctor"
}
finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force -Recurse }
}
