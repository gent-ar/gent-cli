# Offline behavioral checks for the signed Windows installer.
$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
$installer = Join-Path $repo "tools\install.ps1"
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
    & $python (Join-Path $repo "tools\package-release.py") --target-dir $source --out-dir $output `
        --version $version --target $target --format zip --suffix .exe
    if ($LASTEXITCODE -ne 0) { throw "could not package release fixture" }
    $archive = "gent-$version-$target.zip"
    Set-Content -NoNewline -Encoding utf8 (Join-Path $output "$archive.sigstore.json") '{}'
    Set-Content -NoNewline -Encoding utf8 (Join-Path $output "$archive.manifest.json.sigstore.json") '{}'
    Copy-ReleaseAssets $output $fixture $version
}

function Invoke-Installer([string[]]$installerArguments, [bool]$mustSucceed = $true) {
    $oldPreference = $ErrorActionPreference
    $failure = ""
    try {
        $ErrorActionPreference = "Continue"
        try {
            & $installer @installerArguments
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

try {
    Assert-True (Test-Path -LiteralPath $installer -PathType Leaf) "tools/install.ps1 missing"
    $fixture = Join-Path $work "releases"
    $installRoot = Join-Path $work "installed"
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
    try {
        $env:PATH = "$fakeBin;$oldPath"
        $env:GENT_RELEASE_BASE_URL = "$base/v0.1.0"
        Invoke-Installer -installerArguments @("-Version", "v0.1.0", "-InstallDir", $installRoot)
        $runtime = $installRoot
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.1.0-$target") "first release was not activated"
        Assert-True (Test-Path (Join-Path $installRoot "bin\gent.cmd")) "gent launcher missing"
        Assert-True (Test-Path (Join-Path $installRoot "bin\gentd.cmd")) "gentd launcher missing"
        $env:GENT_RELEASE_BASE_URL = "$base/v0.2.0"
        Invoke-Installer -installerArguments @("-Version", "v0.2.0", "-InstallDir", $installRoot) -mustSucceed $false
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.1.0-$target") "failed update changed current"
        Invoke-Installer -installerArguments @("-Version", "v0.2.0", "-InstallDir", $installRoot, "-Force")
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.2.0-$target") "forced update was not activated"
        $env:GENT_RELEASE_BASE_URL = "$base/v0.3.0"
        Invoke-Installer -installerArguments @("-Version", "v0.3.0", "-InstallDir", $installRoot, "-Force") -mustSucceed $false
        Assert-True ((Get-CurrentRelease $runtime) -eq "v0.2.0-$target") "invalid update changed current"
    } finally {
        $env:PATH = $oldPath
        $env:GENT_RELEASE_BASE_URL = $oldBase
    }
    Write-Output "Windows installer checks passed"
} finally {
    if ($null -ne $server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
    if (Test-Path -LiteralPath $work) { Remove-Item -Recurse -Force $work }
}
