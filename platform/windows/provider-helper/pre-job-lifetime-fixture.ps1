param([Parameter(Mandatory=$true)][string]$PauseHelper, [Parameter(Mandatory=$true)][string]$Probe,
      [Parameter(Mandatory=$true)][string]$ArtifactDirectory)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $ArtifactDirectory | Out-Null
$root = Join-Path ([IO.Path]::GetTempPath()) ('gent-prejob-fixture-' + [guid]::NewGuid().ToString('N'))
$workspace = Join-Path $root 'workspace'; $state = Join-Path $root 'state'
New-Item -ItemType Directory -Path $root, $workspace, $state | Out-Null
function Quote-Argument([string]$value) { return '"' + ([regex]::Replace([regex]::Replace($value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1')) + '"' }
$digest = (Get-FileHash -LiteralPath $Probe).Hash.ToLowerInvariant()
$arguments = @('run', '--workspace', $workspace, '--read', $workspace, '--read', $Probe, '--write', $state, '--state-root', $state,
    '--exe', $Probe, '--sha256', $digest, '--profile', ('a' * 64), '--run', 'prejob-fixture', '--processes', '2', '--memory', '67108864', '--cpu-ms', '1000', '--')
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = $PauseHelper; $start.Arguments = ($arguments | ForEach-Object { Quote-Argument $_ }) -join ' '
$start.UseShellExecute = $false; $start.CreateNoWindow = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true; $start.RedirectStandardInput = $true
$helperProcess = New-Object Diagnostics.Process; $helperProcess.StartInfo = $start; $started = $helperProcess.Start()
$receipt = Join-Path $state 'suspended-process.fixture'; $deadline = [DateTime]::UtcNow.AddSeconds(15)
while (-not (Test-Path -LiteralPath $receipt) -and -not $helperProcess.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 25 }
if (-not (Test-Path -LiteralPath $receipt)) { if (-not $helperProcess.HasExited) { $helperProcess.Kill() }; throw ('Suspended receipt unavailable: ' + $helperProcess.StandardError.ReadToEnd()) }
$binding = [IO.File]::ReadAllText($receipt).Split(':'); $childId = [int]$binding[0]; $creation = [long]$binding[1]
$child = Get-Process -Id $childId
if ($child.StartTime.ToFileTimeUtc() -ne $creation) { $helperProcess.Kill(); throw 'Suspended child generation mismatch' }
$helperProcess.Kill(); $helperProcess.WaitForExit()
$terminated = $child.WaitForExit(10000)
$summary = [ordered]@{ root=$root; childPid=$childId; creation=$creation; pausedBeforeJobAssignment=$true;
    helperKilled=$helperProcess.HasExited; psecTerminateOnCloseKilledChild=$terminated; passed=$terminated }
if (-not $terminated) {
    if ($child.StartTime.ToFileTimeUtc() -eq $creation) { $child.Kill(); $child.WaitForExit() }
    $summary['fixtureCleanupKilledExactOwnedChild'] = $true
}
$summary | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $ArtifactDirectory 'summary.json')
$summary | ConvertTo-Json
if (-not $terminated) { throw 'PSEC did not kill the pre-job suspended process when its owner exited.' }
