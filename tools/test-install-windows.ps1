# Offline behavioral checks for the signed Windows installer.
$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
$installer = Join-Path $repo "tools\install.ps1"
$launcherSource = Join-Path $repo "crates\gent-cli\src\windows_launcher.rs"
$python = (Get-Command python -ErrorAction Stop).Source
$target = "x86_64-pc-windows-msvc"
$work = Join-Path ([IO.Path]::GetTempPath()) ("gent-install-test-" + [guid]::NewGuid())
$server = $null

function Assert-True([bool]$condition, [string]$message) {
    if (-not $condition) { throw $message }
}

function Find-FreePort() {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    $listener.Stop()
    return $port
}

function Write-FixtureBinary([string]$path, [string]$name) {
    [IO.File]::WriteAllBytes($path, [Text.Encoding]::UTF8.GetBytes("$name fixture`n"))
}

function Copy-ReleaseAssets([string]$source, [string]$fixture, [string]$version) {
    $nested = Join-Path $fixture $version
    New-Item -ItemType Directory -Path $nested -Force | Out-Null
    Get-ChildItem -File $source | ForEach-Object {
        Copy-Item $_.FullName (Join-Path $fixture $_.Name)
        Copy-Item $_.FullName (Join-Path $nested $_.Name)
    }
}

function New-ReleaseFixture([string]$fixture, [string]$version) {
    $source = Join-Path $work ("source-" + $version)
    $output = Join-Path $work ("output-" + $version)
    New-Item -ItemType Directory -Path $source, $output | Out-Null
    Write-FixtureBinary (Join-Path $source "gent.exe") "gent"
    Write-FixtureBinary (Join-Path $source "gentd.exe") "gentd"
    Write-FixtureBinary (Join-Path $source "gent-launcher.exe") "launcher"
    & $python (Join-Path $repo "tools\package-release.py") --target-dir $source --out-dir $output `
        --version $version --target $target --format zip --suffix .exe
    if ($LASTEXITCODE -ne 0) { throw "could not package release fixture" }
    $archive = "gent-$version-$target.zip"
    Set-Content -NoNewline -Encoding utf8 (Join-Path $output "$archive.sigstore.json") '{}'
    Set-Content -NoNewline -Encoding utf8 (Join-Path $output "$archive.manifest.json.sigstore.json") '{}'
    Copy-Item (Join-Path $repo "tools\gent-auto-update.ps1") (Join-Path $output "gent-auto-update.ps1")
    Set-Content -NoNewline -Encoding utf8 (Join-Path $output "gent-auto-update.ps1.sigstore.json") '{}'
    Copy-ReleaseAssets $output $fixture $version
}

function Invoke-Installer([string]$version, [string]$installRoot, [bool]$force = $false,
    [bool]$mustSucceed = $true, [string]$expected = "", [string]$idleData = "") {
    $oldPreference = $ErrorActionPreference
    $failure = ""
    try {
        $ErrorActionPreference = "Continue"
        try {
            if ($force) {
                if ($expected -and $idleData) {
                    & $installer -Version $version -InstallDir $installRoot -Force -ExpectedSha256 $expected -IdleDataDir $idleData
                } elseif ($expected) {
                    & $installer -Version $version -InstallDir $installRoot -Force -ExpectedSha256 $expected
                } else { & $installer -Version $version -InstallDir $installRoot -Force }
            } elseif ($expected -and $idleData) {
                & $installer -Version $version -InstallDir $installRoot -ExpectedSha256 $expected -IdleDataDir $idleData
            } elseif ($expected) {
                & $installer -Version $version -InstallDir $installRoot -ExpectedSha256 $expected
            } else { & $installer -Version $version -InstallDir $installRoot }
            $code = $LASTEXITCODE
        } catch {
            $code = 1
            $failure = $_.Exception.Message
        }
    } finally {
        $ErrorActionPreference = $oldPreference
    }
    if ($mustSucceed -and $code -ne 0) { throw "installer failed with exit ${code}: $failure" }
    if (-not $mustSucceed -and $code -eq 0) { throw "installer unexpectedly succeeded" }
}

function Get-CurrentRelease([string]$runtime) {
    $pointer = Join-Path $runtime "current.json"
    Assert-True (Test-Path -LiteralPath $pointer -PathType Leaf) "current.json missing"
    $value = Get-Content -Raw -LiteralPath $pointer | ConvertFrom-Json
    Assert-True ($null -ne $value.release) "current.json has no release"
    return [string]$value.release
}

function Archive-Digest([string]$fixture, [string]$version) {
    $archive = Join-Path $fixture "$version\gent-$version-$target.zip"
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
}

try {
    Assert-True (Test-Path -LiteralPath $installer -PathType Leaf) "tools/install.ps1 missing"
    Assert-True (Test-Path -LiteralPath $launcherSource -PathType Leaf) "native launcher source missing"
    $launcherText = Get-Content -Raw -LiteralPath $launcherSource
    Assert-True ($launcherText.Contains("Command::new(target)")) "native launcher must use direct process execution"
    Assert-True ($launcherText.Contains(".args(arguments)")) "native launcher must preserve typed arguments"
    Assert-True (-not $launcherText.Contains("cmd.exe")) "native launcher must not invoke cmd.exe"
    Assert-True ($launcherText.Contains("symlink_metadata")) "native launcher must reject reparse points"
    $installerText = Get-Content -Raw -LiteralPath $installer
    Assert-True ($installerText.Contains("ReparsePoint")) "installer must reject retained release reparse points"
    Assert-True ($installerText.Contains("Test-StagedPair")) "installer must health-check auto-update candidates"
    $fixture = Join-Path $work "releases"
    $installRoot = Join-Path $work "installed"
    $dataRoot = Join-Path $work "data"
    $fakeBin = Join-Path $work "fake-bin"
    New-Item -ItemType Directory -Path $fixture, $fakeBin | Out-Null
    New-ReleaseFixture $fixture "v0.1.0"
    New-ReleaseFixture $fixture "v0.2.0"
    New-ReleaseFixture $fixture "v0.3.0"

    $badManifest = Join-Path $fixture "v0.3.0\gent-v0.3.0-$target.zip.manifest.json"
    $manifest = Get-Content -Raw $badManifest | ConvertFrom-Json
    $manifest.archive.sha256 = "0" * 64
    $manifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 $badManifest
    Set-Content -NoNewline -Encoding utf8 (Join-Path $fixture "v0.3.0\gent-v0.3.0-$target.zip.sha256") `
        (("0" * 64) + "  gent-v0.3.0-$target.zip")

    Set-Content -NoNewline -Encoding ascii (Join-Path $fakeBin "cosign.cmd") "@exit /b 0`r`n"
    $port = Find-FreePort
    $server = Start-Process -FilePath $python -ArgumentList @("-m", "http.server", "$port", "--bind", "127.0.0.1", "--directory", $fixture) -PassThru
    $base = "http://127.0.0.1:$port"
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        try { Invoke-WebRequest -UseBasicParsing "$base/" | Out-Null; break } catch { Start-Sleep -Milliseconds 100 }
    }

    $oldPath = $env:PATH
    $oldBase = $env:GENT_RELEASE_BASE_URL
    $oldScheduler = $env:GENT_AUTO_UPDATE_SCHEDULER_DIR
    try {
        $env:PATH = "$fakeBin;$oldPath"
        $env:GENT_AUTO_UPDATE_SCHEDULER_DIR = Join-Path $work "scheduler"
        $env:GENT_RELEASE_BASE_URL = "$base/v0.1.0"
        $firstDigest = Archive-Digest $fixture "v0.1.0"
        Invoke-Installer -version "v0.1.0" -installRoot $installRoot -expected $firstDigest -idleData $dataRoot
        $runtime = $installRoot
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.1.0-$target") "first release was not activated"
        $pointerBytes = [IO.File]::ReadAllBytes((Join-Path $runtime "current.json"))
        Assert-True ($pointerBytes.Length -gt 0 -and $pointerBytes[0] -eq [byte][char]'{') "current pointer has a UTF-8 BOM"
        Assert-True (Test-Path (Join-Path $installRoot "bin\gent.exe")) "gent launcher missing"
        Assert-True (Test-Path (Join-Path $installRoot "bin\gentd.exe")) "gentd launcher missing"
        Assert-True (Test-Path (Join-Path $installRoot "gent-auto-update.ps1")) "signed automatic-update helper missing"
        Assert-True (Test-Path (Join-Path $env:GENT_AUTO_UPDATE_SCHEDULER_DIR "gent-auto-update.task.json")) "automatic update is not enabled by default"
        Assert-True (-not (Test-Path (Join-Path $installRoot "bin\gent.cmd"))) "unsafe gent.cmd survived"
        Assert-True (-not (Test-Path (Join-Path $installRoot "bin\gentd.cmd"))) "unsafe gentd.cmd survived"
        $env:GENT_RELEASE_BASE_URL = "$base/v0.2.0"
        Invoke-Installer -version "v0.2.0" -installRoot $installRoot -mustSucceed $false
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.1.0-$target") "failed update changed current"
        Set-Content -NoNewline -Encoding ascii (Join-Path $installRoot "bin\gent.cmd") "legacy"
        Set-Content -NoNewline -Encoding ascii (Join-Path $installRoot "bin\gentd.cmd") "legacy"
        Invoke-Installer -version "v0.2.0" -installRoot $installRoot -force $true
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.2.0-$target") "forced update was not activated"
        Assert-True (-not (Test-Path (Join-Path $installRoot "bin\gent.cmd"))) "legacy gent.cmd was not removed"
        Assert-True (-not (Test-Path (Join-Path $installRoot "bin\gentd.cmd"))) "legacy gentd.cmd was not removed"
        Add-Content -NoNewline -Encoding utf8 (Join-Path $installRoot "releases\v0.1.0-$target\gent.exe") "tampered"
        $env:GENT_RELEASE_BASE_URL = "$base/v0.1.0"
        Invoke-Installer -version "v0.1.0" -installRoot $installRoot -force $true -mustSucceed $false
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.2.0-$target") "tampered retained release was activated"
        $env:GENT_RELEASE_BASE_URL = "$base/v0.2.0"
        $lockPath = Join-Path $dataRoot "gentd.lock"
        New-Item -ItemType Directory -Force -Path $dataRoot | Out-Null
        $lock = [System.IO.File]::Open($lockPath, "OpenOrCreate", "ReadWrite", "ReadWrite")
        try {
            $lock.Lock(0, 1)
            $secondDigest = Archive-Digest $fixture "v0.2.0"
            Invoke-Installer -version "v0.2.0" -installRoot $installRoot -force $true -mustSucceed $false `
                -expected $secondDigest -idleData $dataRoot
            Assert-True ((Get-CurrentRelease $runtime) -eq "v0.2.0-$target") "idle-lock refusal changed current"
        } finally { $lock.Unlock(0, 1); $lock.Dispose() }
        $env:GENT_RELEASE_BASE_URL = "$base/v0.3.0"
        Invoke-Installer -version "v0.3.0" -installRoot $installRoot -force $true -mustSucceed $false
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.2.0-$target") "invalid update changed current"
    } finally {
        $env:PATH = $oldPath
        $env:GENT_RELEASE_BASE_URL = $oldBase
        $env:GENT_AUTO_UPDATE_SCHEDULER_DIR = $oldScheduler
    }
    Write-Output "Windows installer checks passed"
} finally {
    if ($null -ne $server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
    if (Test-Path -LiteralPath $work) { Remove-Item -Recurse -Force $work }
}
