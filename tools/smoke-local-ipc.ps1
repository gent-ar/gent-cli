# Verifies the Windows named-pipe runtime slice without a provider process.
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$dataDir = Join-Path ([System.IO.Path]::GetTempPath()) ("gent-smoke-" + [guid]::NewGuid())
$daemon = $null

function Assert-Equal([object]$actual, [object]$expected, [string]$label) {
    if ($actual -ne $expected) {
        throw "${label}: expected '$expected', got '$actual'"
    }
}

try {
    New-Item -ItemType Directory -Path $dataDir | Out-Null
    Push-Location $root
    cargo build --quiet -p gentd -p gent-cli
    $gentd = Join-Path $root "target\debug\gentd.exe"
    $gent = Join-Path $root "target\debug\gent.exe"
    $daemon = Start-Process -FilePath $gentd -ArgumentList @("--data-dir", $dataDir) -PassThru

    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        $statusJson = & $gent --data-dir $dataDir status 2>$null
        if ($LASTEXITCODE -eq 0) { break }
        Start-Sleep -Milliseconds 50
    }
    if ($LASTEXITCODE -ne 0) { throw "gentd did not become ready" }

    $status = $statusJson | ConvertFrom-Json
    $receipt = & $gent --data-dir $dataDir submit --kind ping --payload '{"message":"smoke"}' | ConvertFrom-Json
    $events = & $gent --data-dir $dataDir events | ConvertFrom-Json
    $decision = & $gent --data-dir $dataDir decision submit --decision-id smoke-decision --idempotency-key smoke-key | ConvertFrom-Json
    $terminal = & $gent --data-dir $dataDir decision unprovable --decision-id smoke-decision | ConvertFrom-Json

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
    }
    if (Test-Path $dataDir) { Remove-Item -Recurse -Force $dataDir }
    Pop-Location -ErrorAction SilentlyContinue
}
