# Verifies the Windows named-pipe runtime slice without a provider process.
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$dataDir = Join-Path ([System.IO.Path]::GetTempPath()) ("gent-smoke-" + [guid]::NewGuid())
$daemon = $null
$defaultDaemon = $null
$gent = $null
$originalHome = $env:HOME
$originalUserProfile = $env:USERPROFILE

function Assert-Equal([object]$actual, [object]$expected, [string]$label) {
    if ($actual -ne $expected) {
        throw "${label}: expected '$expected', got '$actual'"
    }
}

function Invoke-Gent([string]$label, [string[]]$arguments) {
    $fullArguments = @("--no-autostart") + $arguments
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $gent
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.Arguments = ($fullArguments | ForEach-Object {
        '"' + $_.Replace('"', '\"') + '"'
    }) -join ' '
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $null = $process.Start()
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(10000)) {
        $process.Kill($true)
        $daemonLog = Join-Path $dataDir "gentd.stderr"
        $details = if (Test-Path $daemonLog) { Get-Content -Raw $daemonLog } else { "no daemon log" }
        throw "${label} timed out; daemon stderr: $details"
    }
    $output = $stdoutTask.GetAwaiter().GetResult()
    $errors = $stderrTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) {
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
    $defaultHome = Join-Path $dataDir "home"
    New-Item -ItemType Directory -Path $defaultHome | Out-Null
    $env:HOME = $defaultHome
    $env:USERPROFILE = $defaultHome
    $defaultDaemon = Start-Process -FilePath $gentd -ArgumentList @("--standalone-authority") -PassThru `
        -RedirectStandardOutput (Join-Path $dataDir "default-gentd.stdout") `
        -RedirectStandardError (Join-Path $dataDir "default-gentd.stderr")
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        try {
            $defaultStatusJson = Invoke-Gent "default status" @("status")
            break
        } catch {
            if ($attempt -eq 39) { throw }
        }
        Start-Sleep -Milliseconds 50
    }
    $defaultStatus = $defaultStatusJson | ConvertFrom-Json
    Assert-Equal $defaultStatus.type "status" "default status frame"
    Stop-Process -Id $defaultDaemon.Id -Force
    $null = $defaultDaemon.WaitForExit(5000)
    $defaultDaemon = $null
    $env:HOME = $originalHome
    $env:USERPROFILE = $originalUserProfile
    $daemon = Start-Process -FilePath $gentd -ArgumentList @("--data-dir", $dataDir, "--standalone-authority") -PassThru `
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
    $created = Invoke-Gent "chat create" @("--data-dir", $dataDir, "chat", "create", "--provider", "claude", "--model", "haiku", "--effort", "medium", "--mode", "ask") | ConvertFrom-Json
    $conversations = Invoke-Gent "conversation list" @("--data-dir", $dataDir, "conversation", "list") | ConvertFrom-Json

    Assert-Equal $status.type "status" "status frame"
    Assert-Equal $receipt.body.status "settled" "receipt status"
    $eventKinds = ($events.body.page.events | ForEach-Object { $_.kind }) -join ","
    Assert-Equal $eventKinds "commandAccepted,commandSettled" "event order"
    Assert-Equal $decision.type "decisionSubmission" "decision frame"
    Assert-Equal $decision.body.outcome "accepted" "decision outcome"
    Assert-Equal $terminal.body.phase "unprovable" "decision terminal phase"
    Assert-Equal $created.body.receipt.status "settled" "chat creation receipt"
    Assert-Equal $conversations.Count 1 "conversation count"
    Assert-Equal $conversations[0].conversationId $created.body.conversationId "conversation identity"
}
finally {
    if ($null -ne $daemon -and -not $daemon.HasExited) {
        Stop-Process -Id $daemon.Id -Force
        $null = $daemon.WaitForExit(5000)
    }
    if ($null -ne $defaultDaemon -and -not $defaultDaemon.HasExited) {
        Stop-Process -Id $defaultDaemon.Id -Force
        $null = $defaultDaemon.WaitForExit(5000)
    }
    $env:HOME = $originalHome
    $env:USERPROFILE = $originalUserProfile
    if (Test-Path $dataDir) {
        try {
            Remove-Item -Recurse -Force $dataDir
        } catch {
            Write-Warning "could not remove smoke data directory: $dataDir"
        }
    }
    Pop-Location -ErrorAction SilentlyContinue
}
