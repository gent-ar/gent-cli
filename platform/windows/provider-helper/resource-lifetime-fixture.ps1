param([Parameter(Mandatory=$true)][string]$Helper, [Parameter(Mandatory=$true)][string]$Probe,
      [Parameter(Mandatory=$true)][string]$ArtifactDirectory, [string]$LanHost='', [int]$LanPort=45683)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $ArtifactDirectory | Out-Null
$root = Join-Path ([IO.Path]::GetTempPath()) ('gent-resource-fixture-' + [guid]::NewGuid().ToString('N'))
$workspace = Join-Path $root 'workspace'; $state = Join-Path $root 'state'
New-Item -ItemType Directory -Path $root, $workspace, $state | Out-Null
function Quote-Argument([string]$value) { return '"' + ([regex]::Replace([regex]::Replace($value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1')) + '"' }
$digest = (Get-FileHash -LiteralPath $Probe).Hash.ToLowerInvariant()
function Start-Contained([string[]]$Payload, [int]$Processes, [long]$Memory, [switch]$Gateway) {
    $arguments = @('run', '--workspace', $workspace, '--read', $workspace, '--read', $Probe, '--read', (Join-Path $env:SystemRoot 'System32/mswsock.dll'), '--write', $workspace, '--write', $state, '--state-root', $state,
        '--exe', $Probe, '--sha256', $digest, '--profile', ('a' * 64), '--run', 'resource-fixture', '--processes', "$Processes", '--memory', "$Memory", '--cpu-ms', '30000')
    if ($Gateway) { $script:peerNonce = [guid]::NewGuid().ToString('N'); $arguments += @('--peer-nonce', $script:peerNonce, '--allow-loopback-port', '45679', '--gateway-api', 'claude', '--gateway-url', 'http://127.0.0.1:45679', '--gateway-token', ('b' * 64)) }
    $arguments += @('--') + $Payload
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $Helper; $start.Arguments = ($arguments | ForEach-Object { Quote-Argument $_ }) -join ' '
    $start.UseShellExecute = $false; $start.CreateNoWindow = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true; $start.RedirectStandardInput = $true
    $process = New-Object Diagnostics.Process; $process.StartInfo = $start; $started = $process.Start(); $process.StandardInput.Close(); return $process
}
& $Probe memory-check allowed
if ($LASTEXITCODE -ne 0) { throw 'Native memory control failed' }
$memory = Start-Contained -Payload @('memory-check', 'denied') -Processes 1 -Memory 16777216
if (-not $memory.WaitForExit(15000)) { $memory.Kill(); throw 'Memory fixture timed out' }
$memoryOutput = $memory.StandardOutput.ReadToEnd(); $memoryError = $memory.StandardError.ReadToEnd()
$memoryPassed = $memory.ExitCode -eq 0 -and $memoryOutput.Contains('memory-denied') -and $memoryError.Length -eq 0
$marker = Join-Path $workspace 'child-marker.txt'
& $Probe process-check $marker allowed
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $marker)) { throw 'Native child-process control failed' }
Remove-Item -LiteralPath $marker
$childControl = Start-Contained -Payload @('process-check', $marker, 'allowed') -Processes 2 -Memory 67108864
if (-not $childControl.WaitForExit(15000)) { $childControl.Kill(); throw 'Contained child positive control timed out' }
if ($childControl.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $marker)) { throw ('Contained child positive control failed: ' + $childControl.StandardError.ReadToEnd()) }
Remove-Item -LiteralPath $marker
$processes = Start-Contained -Payload @('process-check', $marker, 'denied') -Processes 1 -Memory 67108864
if (-not $processes.WaitForExit(15000)) { $processes.Kill(); throw 'Process-count fixture timed out' }
$processOutput = $processes.StandardOutput.ReadToEnd(); $processError = $processes.StandardError.ReadToEnd()
$processPassed = $processes.ExitCode -eq 0 -and $processOutput.Contains('child-denied') -and -not (Test-Path -LiteralPath $marker) -and $processError.Length -eq 0
$lanPassed = $null
if ($LanHost) {
    & $Probe network-check $LanHost "$LanPort" allowed
    if ($LASTEXITCODE -ne 0) { throw 'Native LAN egress control unavailable' }
    $lan = Start-Contained -Payload @('network-check', $LanHost, "$LanPort", 'denied') -Processes 4 -Memory 134217728 -Gateway
    if (-not $lan.WaitForExit(15000)) { $lan.Kill(); throw 'LAN egress fixture timed out' }
    $lanOutput = $lan.StandardOutput.ReadToEnd(); $lanError = $lan.StandardError.ReadToEnd()
    $lanPassed = $lan.ExitCode -eq 0 -and $lanOutput.Contains('network-error:10013') -and $lanError.Length -eq 0
    $lanSummary = [ordered]@{ helperSha256=(Get-FileHash -LiteralPath $Helper).Hash.ToLowerInvariant(); probeSha256=$digest; host=$LanHost; port=$LanPort; nativeControlPassed=$true; containedExit=$lan.ExitCode; containedOutput=$lanOutput; passed=$lanPassed }
    $lanSummary | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $ArtifactDirectory 'lan-egress.json')
    if (-not $lanPassed) { $lanSummary | ConvertTo-Json; throw 'Private LAN direct egress was not denied by the gateway sandbox' }
}
$trace = Join-Path $workspace 'tree.txt'
$tree = Start-Contained -Payload @('tree-run', $trace) -Processes 4 -Memory 134217728 -Gateway
$headerTask = $tree.StandardOutput.ReadLineAsync(); $deadline = [DateTime]::UtcNow.AddSeconds(15)
while (-not $headerTask.IsCompleted -and -not $tree.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 25 }
if (-not $headerTask.IsCompleted) { $tree.Kill(); throw 'Tree fixture has no native proof' }
$header = $headerTask.Result
if ($header -notmatch ' peer=(GentGateway-[a-f0-9]{32}),([0-9]+),([0-9]+),([0-9]+)$') { if (-not $tree.HasExited) { $tree.Kill() }; throw ('Tree native proof rejected: ' + $tree.StandardError.ReadToEnd()) }
$peerName = $Matches[1]; $peerId = [int]$Matches[3]; $peerCreation = [long]$Matches[4]
if ($peerName -ne "GentGateway-$script:peerNonce") { $tree.Kill(); throw 'Peer nonce mismatch' }
$deadline = [DateTime]::UtcNow.AddSeconds(10); $bindings = @()
while ($bindings.Count -lt 3 -and -not $tree.HasExited -and [DateTime]::UtcNow -lt $deadline) {
    if (Test-Path -LiteralPath $trace) { $bindings = @([IO.File]::ReadAllLines($trace) | Where-Object { $_ -match '^[0-9]+:[0-9]+$' }) }
    Start-Sleep -Milliseconds 25
}
if ($bindings.Count -ne 3) { if (-not $tree.HasExited) { $tree.Kill(); $tree.WaitForExit() }; throw ('Three-level process tree was not established; count=' + $bindings.Count + '; exit=' + $tree.ExitCode + '; stderr=' + $tree.StandardError.ReadToEnd()) }
$owned = @()
foreach ($binding in ($bindings + "$peerId`:$peerCreation")) {
    $parts = $binding.Split(':'); $ownedProcess = Get-Process -Id ([int]$parts[0])
    if ($ownedProcess.StartTime.ToFileTimeUtc() -ne [long]$parts[1]) { $tree.Kill(); throw 'Live tree generation mismatch' }
    $owned += $ownedProcess
}
$savedErrorAction = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
try { $liveCleanup = & $Helper cleanup-peer $state $peerName "$peerId" "$peerCreation" 2>&1; $liveCode = $LASTEXITCODE } finally { $ErrorActionPreference = $savedErrorAction }
$liveRefused = $liveCode -eq 126 -and (Test-Path -LiteralPath (Join-Path $state $peerName))
$tree.Kill(); $tree.WaitForExit(); $allTerminated = $true
foreach ($ownedProcess in $owned) { if (-not $ownedProcess.WaitForExit(5000)) { $allTerminated = $false; $ownedProcess.Kill(); $ownedProcess.WaitForExit() } }
$pending = & $Helper cleanup-pending $state $peerName 2>&1
$cleanupPassed = $LASTEXITCODE -eq 0 -and -not (Test-Path -LiteralPath (Join-Path $state $peerName))
$summary = [ordered]@{ root=$root; helperSha256=(Get-FileHash -LiteralPath $Helper).Hash.ToLowerInvariant(); probeSha256=$digest; memoryDenied=$memoryPassed; processCountDenied=$processPassed; threeLevelTreeEstablished=$true; peerGeneration=$peerCreation;
    privateLanEgressDenied=$lanPassed;
    livePeerCleanupRefused=$liveRefused; helperKillTerminatedTreeAndPeer=$allTerminated; pendingPeerCleanup=$cleanupPassed;
    passed=($memoryPassed -and $processPassed -and $liveRefused -and $allTerminated -and $cleanupPassed) }
$summary | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $ArtifactDirectory 'summary.json')
$summary | ConvertTo-Json
if (-not $summary.passed) { throw 'Native resource/lifetime fixture failed' }
