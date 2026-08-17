# Offline behavior checks for the signed Windows automatic-update helper.
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$helper = Join-Path $repo "tools\gent-auto-update.ps1"
$python = (Get-Command python -ErrorAction Stop).Source
$work = Join-Path ([IO.Path]::GetTempPath()) ("gent-auto-update-test-" + [guid]::NewGuid())
$server = $null

function Assert-True([bool]$Value, [string]$Message) { if (-not $Value) { throw $Message } }
function Free-Port {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0); $listener.Start()
    $port = ([Net.IPEndPoint]$listener.LocalEndpoint).Port; $listener.Stop(); return $port
}
function Invoke-Helper([string[]]$Arguments, [bool]$MustSucceed = $true) {
    & powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $helper @Arguments
    if ($MustSucceed -and $LASTEXITCODE -ne 0) { throw "automatic updater failed" }
    if (-not $MustSucceed -and $LASTEXITCODE -eq 0) { throw "automatic updater unexpectedly succeeded" }
}

try {
    Assert-True (Test-Path -LiteralPath $helper -PathType Leaf) "Windows automatic-update helper missing"
    $text = Get-Content -Raw -LiteralPath $helper
    Assert-True ($text.Contains("Register-ScheduledTask")) "helper must register a scheduled task"
    Assert-True ($text.Contains("verify-blob")) "helper must verify its tag-bound bootstrap"
    Assert-True ($text.Contains("RequireHealth")) "helper must require staged runtime health"
    $runtime = Join-Path $work "runtime"; $release = Join-Path $runtime "releases\v1.2.3-x86_64-pc-windows-msvc"
    $files = Join-Path $work "files"; $fake = Join-Path $work "fake"; $scheduler = Join-Path $work "scheduler"; $record = Join-Path $work "record"
    New-Item -ItemType Directory -Force -Path $release, $files, $fake | Out-Null
    Set-Content -NoNewline -Encoding utf8 (Join-Path $release "gent.exe") "fixture"
    Set-Content -NoNewline -Encoding utf8 (Join-Path $release "gentd.exe") "fixture"
    @{ release = "v1.2.3-x86_64-pc-windows-msvc" } | ConvertTo-Json -Compress | Set-Content -NoNewline -Encoding utf8 (Join-Path $runtime "current.json")
    $tagDir = Join-Path $files "v1.2.4"; New-Item -ItemType Directory -Force -Path $tagDir | Out-Null
    @{ tag_name = "v1.2.4"; draft = $false; prerelease = $false } | ConvertTo-Json -Compress | Set-Content -NoNewline -Encoding utf8 (Join-Path $files "latest")
    @'
param([string]$Version,[string]$InstallDir,[string]$IdleDataDir,[switch]$Force,[switch]$RequireHealth)
[IO.File]::WriteAllText($env:GENT_TEST_RECORD, "$Version|$InstallDir|$IdleDataDir|$Force|$RequireHealth")
'@ | Set-Content -NoNewline -Encoding utf8 (Join-Path $tagDir "gent-install.ps1")
    Set-Content -NoNewline -Encoding utf8 (Join-Path $tagDir "gent-install.ps1.sigstore.json") "{}"
    Set-Content -NoNewline -Encoding ascii (Join-Path $fake "cosign.cmd") "@exit /b 0`r`n"
    $port = Free-Port; $server = Start-Process -FilePath $python -ArgumentList @("-m", "http.server", "$port", "--bind", "127.0.0.1", "--directory", $files) -PassThru
    $base = "http://127.0.0.1:$port"; for ($attempt = 0; $attempt -lt 30; $attempt++) { try { Invoke-WebRequest -UseBasicParsing "$base/" | Out-Null; break } catch { Start-Sleep -Milliseconds 100 } }
    $oldPath = $env:PATH; $oldApi = $env:GENT_RELEASE_API_URL; $oldBase = $env:GENT_RELEASE_DOWNLOAD_BASE_URL; $oldRecord = $env:GENT_TEST_RECORD
    try {
        $env:PATH = "$fake;$oldPath"; $env:GENT_RELEASE_API_URL = "$base/latest"; $env:GENT_RELEASE_DOWNLOAD_BASE_URL = $base; $env:GENT_TEST_RECORD = $record
        Invoke-Helper @("enable", "-RuntimeRoot", $runtime, "-DataDir", (Join-Path $work "data"), "-SchedulerDir", $scheduler, "-IntervalSeconds", "600")
        Assert-True (Test-Path (Join-Path $scheduler "gent-auto-update.task.ps1")) "scheduler wrapper missing"
        $status = & powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $helper status -RuntimeRoot $runtime -DataDir (Join-Path $work "data") -SchedulerDir $scheduler | ConvertFrom-Json
        Assert-True $status.enabled "scheduler status is not enabled"
        $result = & powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $helper run -RuntimeRoot $runtime -DataDir (Join-Path $work "data") | ConvertFrom-Json
        Assert-True ($result.result -eq "updated") "newer verified tag was not delegated"
        Assert-True ((Get-Content -Raw $record) -match 'v1.2.4\|.*\|.*\|True\|True') "installer lacks forced staged-health delegation"
        Invoke-Helper @("disable", "-RuntimeRoot", $runtime, "-DataDir", (Join-Path $work "data"), "-SchedulerDir", $scheduler)
        Assert-True (-not (Test-Path (Join-Path $scheduler "gent-auto-update.task.json"))) "scheduler did not clean up"
    } finally { $env:PATH = $oldPath; $env:GENT_RELEASE_API_URL = $oldApi; $env:GENT_RELEASE_DOWNLOAD_BASE_URL = $oldBase; $env:GENT_TEST_RECORD = $oldRecord }
    Write-Output "Windows automatic-update checks passed"
} finally {
    if ($null -ne $server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
    if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Force -Recurse }
}
