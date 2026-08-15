# Verifies the Windows named-pipe runtime slice without a provider process.
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$dataDir = Join-Path ([System.IO.Path]::GetTempPath()) ("gent-smoke-" + [guid]::NewGuid())
$daemon = $null
$gent = $null

function Assert-Equal([object]$actual, [object]$expected, [string]$label) {
    if ($actual -ne $expected) {
        throw "${label}: expected '$expected', got '$actual'"
    }
}

function Invoke-Gent([string]$label, [string[]]$arguments) {
    $stdout = Join-Path $dataDir "${label}.stdout"
    $stderr = Join-Path $dataDir "${label}.stderr"
    $process = Start-Process -FilePath $gent -ArgumentList (@("--no-autostart") + $arguments) -PassThru -NoNewWindow `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    if (-not $process.WaitForExit(10000)) {
        Stop-Process -Id $process.Id -Force
        $daemonLog = Join-Path $dataDir "gentd.stderr"
        $details = if (Test-Path $daemonLog) { Get-Content -Raw $daemonLog } else { "no daemon log" }
        throw "${label} timed out; daemon stderr: $details"
    }
    $output = if (Test-Path $stdout) { Get-Content -Raw $stdout } else { "" }
    if ($process.ExitCode -ne 0) {
        $errors = if (Test-Path $stderr) { Get-Content -Raw $stderr } else { "no stderr" }
        throw "${label} failed with exit $($process.ExitCode): $errors"
    }
    return $output
}

try {
    New-Item -ItemType Directory -Path $dataDir | Out-Null
    Push-Location $root
    cargo build --quiet -p gentd -p gent-cli
    $gentd = Join-Path $root "target\debug\gentd.exe"
    $gent = Join-Path $root "target\debug\gent.exe"
    $daemon = Start-Process -FilePath $gentd -ArgumentList @("--data-dir", $dataDir) -PassThru `
        -RedirectStandardOutput (Join-Path $dataDir "gentd.stdout") `
        -RedirectStandardError (Join-Path $dataDir "gentd.stderr")

    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        try {
            $statusJson = Invoke-Gent "status" @("--data-dir", $dataDir, "status")
            break
        } catch {
            if ($attempt -eq 39) { throw }
        }
        Start-Sleep -Milliseconds 50
    }

    $status = $statusJson | ConvertFrom-Json
    $receipt = Invoke-Gent "receipt" @("--data-dir", $dataDir, "submit", "--kind", "ping") | ConvertFrom-Json
    $events = Invoke-Gent "events" @("--data-dir", $dataDir, "events") | ConvertFrom-Json
    $decision = Invoke-Gent "decision" @("--data-dir", $dataDir, "decision", "submit", "--decision-id", "smoke-decision", "--idempotency-key", "smoke-key") | ConvertFrom-Json
    $terminal = Invoke-Gent "terminal" @("--data-dir", $dataDir, "decision", "unprovable", "--decision-id", "smoke-decision") | ConvertFrom-Json

    Assert-Equal $status.type "status" "status frame"
    Assert-Equal $receipt.body.status "settled" "receipt status"
    $eventKinds = ($events.body.events | ForEach-Object { $_.kind }) -join ","
    Assert-Equal $eventKinds "commandAccepted,commandSettled" "event order"
    Assert-Equal $decision.type "decisionSubmission" "decision frame"
    Assert-Equal $decision.body.outcome "accepted" "decision outcome"
    Assert-Equal $terminal.body.phase "unprovable" "decision terminal phase"
}
finally {
    if ($null -ne $daemon -and -not $daemon.HasExited) {
        Stop-Process -Id $daemon.Id -Force
        $daemon.WaitForExit(5000)
    }
    if (Test-Path $dataDir) {
        try {
            Remove-Item -Recurse -Force $dataDir
        } catch {
            Write-Warning "could not remove smoke data directory: $dataDir"
        }
    }
    Pop-Location -ErrorAction SilentlyContinue
}
