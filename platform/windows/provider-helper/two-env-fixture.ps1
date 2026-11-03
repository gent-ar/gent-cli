param([Parameter(Mandatory=$true)][string]$BuildDirectory, [Parameter(Mandatory=$true)][string]$ManagedRuntime,
      [Parameter(Mandatory=$true)][string]$ArtifactDirectory, [int]$GatewayPort=45779, [int]$UnrelatedPort=45780, [int]$LanPort=45781,
      [string]$PublicAddress='1.1.1.1')
$ErrorActionPreference = 'Stop'
$root = Join-Path $ArtifactDirectory ('run-' + [guid]::NewGuid().ToString('N').Substring(0,12))
$runtime = Join-Path $root 'runtime'; $workspace = Join-Path $root 'workspace'; $outside = Join-Path $root 'outside'; $state = Join-Path $root 'state'
foreach ($directory in @($runtime,$workspace,$outside,$state,(Join-Path $runtime 'powershell-launcher'),(Join-Path $state 'home\.claurst'),(Join-Path $state 'claude'),(Join-Path $state 'codex'))) { New-Item -ItemType Directory -Force -Path $directory | Out-Null }
Copy-Item -Recurse -LiteralPath (Join-Path $ManagedRuntime 'git') -Destination (Join-Path $runtime 'git')
Copy-Item -Recurse -LiteralPath (Join-Path $ManagedRuntime 'powershell') -Destination (Join-Path $runtime 'powershell')
Copy-Item -LiteralPath (Join-Path $BuildDirectory 'gent-windows-sandbox.exe') -Destination (Join-Path $runtime 'helper.exe')
Copy-Item -LiteralPath (Join-Path $BuildDirectory 'gent-two-env-probe.exe') -Destination (Join-Path $runtime 'probe.exe')
foreach ($shim in @('bash.exe','pwsh.exe','cmd.exe')) { Copy-Item -LiteralPath (Join-Path $BuildDirectory $shim) -Destination (Join-Path $runtime "powershell-launcher\$shim") }
[IO.File]::WriteAllText((Join-Path $outside 'secret.txt'),'fixture-only-secret')
foreach ($marker in @('home\.claurst\marker.txt','claude\marker.txt','codex\marker.txt')) { [IO.File]::WriteAllText((Join-Path $state $marker),'marker') }
function Hash([string]$path) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Quote-Argument([string]$value) { '"' + ([regex]::Replace([regex]::Replace($value,'(\\*)"','$1$1\"'),'(\\+)$','$1$1')) + '"' }
$probe = Join-Path $runtime 'probe.exe'; $bash = Join-Path $runtime 'git\usr\bin\bash.exe'; $pwsh = Join-Path $runtime 'powershell\pwsh.exe'; $launcher = Join-Path $runtime 'powershell-launcher\pwsh.exe'
$lan = (Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' -and $_.AddressState -eq 'Preferred' } | Select-Object -First 1).IPAddress
if (-not $lan) { throw 'Host LAN control address is unavailable' }
$gateway = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback,$GatewayPort); $gateway.Start()
$unrelated = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback,$UnrelatedPort); $unrelated.Start()
$lanListener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Parse($lan),$LanPort); $lanListener.Start()
$controls = [ordered]@{}
foreach ($target in @(@('hostUnrelated','127.0.0.1',$UnrelatedPort),@('hostLan',$lan,$LanPort),@('hostPublic',$PublicAddress,443))) {
    $client = New-Object Net.Sockets.TcpClient
    try { $controls[$target[0]] = $client.ConnectAsync($target[1],[int]$target[2]).Wait(5000) } catch { $controls[$target[0]] = $false } finally { $client.Close() }
}
$readLeaves = @($workspace,(Join-Path $env:SystemRoot 'System32\mswsock.dll'),(Join-Path $env:SystemRoot 'System32\conhost.exe')) + @(Get-ChildItem -Recurse -File -LiteralPath $runtime | ForEach-Object { $_.FullName })
$arguments = @('run','--workspace',$workspace,'--write',$workspace,'--read-stdin','1',
    '--write',$state,'--state-root',$state,'--peer-nonce',[guid]::NewGuid().ToString('N'),'--allow-loopback-port',"$GatewayPort",'--gateway-api','claude',
    '--gateway-url',"http://127.0.0.1:$GatewayPort",'--gateway-token-stdin','1',
    '--shell-exe',$bash,'--shell-sha256',(Hash $bash),'--powershell-exe',$pwsh,'--powershell-sha256',(Hash $pwsh),
    '--powershell-launcher-exe',$launcher,'--powershell-launcher-sha256',(Hash $launcher),
    '--exe',$probe,'--sha256',(Hash $probe),'--profile',('a'*64),'--run','two-env-run','--processes','64','--memory','4294967296','--cpu-ms','600000',
    '--','provider',$workspace,$outside,$PublicAddress,"$UnrelatedPort",$lan,"$LanPort",$probe,$runtime,$launcher,"$GatewayPort")
$start = New-Object Diagnostics.ProcessStartInfo; $start.FileName = (Join-Path $runtime 'helper.exe')
$start.Arguments = ($arguments | ForEach-Object { Quote-Argument $_ }) -join ' '
$start.UseShellExecute = $false; $start.CreateNoWindow = $true; $start.RedirectStandardInput = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
$process = New-Object Diagnostics.Process; $process.StartInfo = $start; $null = $process.Start()
$outTask = $process.StandardOutput.ReadToEndAsync(); $errTask = $process.StandardError.ReadToEndAsync()
$frame = New-Object IO.MemoryStream
$ascii = [Text.Encoding]::ASCII.GetBytes('GENTGW01' + ('c'*64) + 'GENTRO01'); $frame.Write($ascii,0,$ascii.Length)
$count = [BitConverter]::GetBytes([uint32]$readLeaves.Count); $frame.Write($count,0,4)
foreach ($leaf in $readLeaves) { $bytes = [Text.Encoding]::UTF8.GetBytes($leaf); $size = [BitConverter]::GetBytes([uint32]$bytes.Length); $frame.Write($size,0,4); $frame.Write($bytes,0,$bytes.Length) }
$frameBytes = $frame.ToArray(); $process.StandardInput.BaseStream.Write($frameBytes,0,$frameBytes.Length); $process.StandardInput.BaseStream.Flush()
$sleeperPid = $null; $sleeperDead = $null; $backgroundPid = $null; $backgroundAliveAfterShell = $null; $gatewayRequests = 0
$deadline = [DateTime]::UtcNow.AddSeconds(240)
while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline) {
    if ($gateway.Pending()) {
        $client = $gateway.AcceptTcpClient(); $stream = $client.GetStream(); $stream.ReadTimeout = 5000; $header = ''; $single = New-Object byte[] 1
        while (-not $header.EndsWith("`r`n`r`n") -and $header.Length -lt 32768 -and $stream.Read($single,0,1) -eq 1) { $header += [char]$single[0] }
        $response = [Text.Encoding]::ASCII.GetBytes("HTTP/1.1 200 OK`r`nContent-Length: 2`r`nConnection: close`r`n`r`nOK"); $stream.Write($response,0,$response.Length); $client.Close(); $gatewayRequests++
    }
    if ($null -eq $sleeperDead -and (Test-Path -LiteralPath (Join-Path $workspace 'shim-killed'))) {
        if (-not (Test-Path -LiteralPath (Join-Path $workspace 'sleeper.pid'))) { $sleeperPid = 0 } else { $sleeperPid = [int][IO.File]::ReadAllText((Join-Path $workspace 'sleeper.pid')) }
        $goneBy = [DateTime]::UtcNow.AddSeconds(10); while ((Get-Process -Id $sleeperPid -ErrorAction SilentlyContinue) -and [DateTime]::UtcNow -lt $goneBy) { Start-Sleep -Milliseconds 50 }
        $sleeperDead = $sleeperPid -ne 0 -and -not (Get-Process -Id $sleeperPid -ErrorAction SilentlyContinue)
    }
    if ($null -eq $backgroundAliveAfterShell -and (Test-Path -LiteralPath (Join-Path $workspace 'background.pid'))) {
        Start-Sleep -Milliseconds 500
        $backgroundPid = [int][IO.File]::ReadAllText((Join-Path $workspace 'background.pid'))
        $backgroundAliveAfterShell = [bool](Get-Process -Id $backgroundPid -ErrorAction SilentlyContinue)
        [IO.File]::WriteAllText((Join-Path $workspace 'host-checked'),'x')
    }
    Start-Sleep -Milliseconds 25
}
$timedOut = -not $process.HasExited
if ($timedOut) { $process.Kill() }
$process.WaitForExit(); $helperExit = $process.ExitCode; $process.StandardInput.Close()
$gateway.Stop(); $unrelated.Stop(); $lanListener.Stop()
$backgroundGoneAfterHelper = $null -ne $backgroundPid -and -not (Get-Process -Id $backgroundPid -ErrorAction SilentlyContinue)
$survivors = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($root,[StringComparison]::OrdinalIgnoreCase) })
$reportPath = Join-Path $workspace 'provider-report.txt'
$report = if (Test-Path -LiteralPath $reportPath) { [IO.File]::ReadAllText($reportPath) } else { '' }
$values = @{}; foreach ($line in ($report -split "`r?`n")) { $at = $line.IndexOf('='); if ($at -gt 0) { $values[$line.Substring(0,$at)] = $line.Substring($at + 1) } }
$secretHeld = [IO.File]::ReadAllText((Join-Path $outside 'secret.txt')) -ceq 'fixture-only-secret'
$checks = [ordered]@{
    hostControlsReachable = $controls.hostUnrelated -and $controls.hostLan -and $controls.hostPublic
    providerReachesGateway = $values['A.gateway_status'] -eq 'HTTP/1.1 200 OK' -and $gatewayRequests -ge 1
    providerNoPublicEgress = $values['A.public_tcp'] -ne '0' -and $values['A.unrelated_loopback'] -ne '0'
    commandPublicTcpDnsHttp = $values['B.public_tcp'] -eq '0' -and $values['B.dns'] -eq '0' -and $values['B.curl_http'] -eq '200/18765'
    commandHasNoGatewayEnvironment = $values['A.benv_read'] -eq 'yes' -and $values['A.benv_token_absent'] -eq 'yes' -and $values['A.benv_origin_absent'] -eq 'yes' -and $values['A.benv_prefix_absent'] -eq 'yes'
    commandCannotReachGateway = $values['B.gateway_proxy_port'] -ne '0' -and $values['B.host_gateway_port'] -ne '0' -and $values['B.curl_gateway'] -match '(^| )000$'
    commandCannotReachHostLocal = $values['B.unrelated_loopback'] -ne '0' -and $values['B.host_lan'] -ne '0'
    commandCannotTouchOutside = $values['B.outside_read'] -eq 'denied' -and $values['B.outside_write'] -eq 'denied' -and $secretHeld
    commandCannotReachProvider = $values['B.open_provider_process'] -eq 'denied' -and $values['B.broker_pipe'] -eq 'denied'
    providerConfigReadOnlyForCommand = $values['B.claude_config_read'] -eq 'allowed' -and $values['B.claude_config_write'] -eq 'denied' -and $values['B.codex_config_read'] -eq 'allowed' -and $values['B.codex_config_write'] -eq 'denied' -and $values['B.claurst_config_read'] -eq 'allowed' -and $values['B.claurst_config_write'] -eq 'denied'
    commandPrivateWrites = $values['B.home_write'] -eq 'allowed' -and $values['B.tmp_write'] -eq 'allowed' -and $values['B.os_temp_write'] -eq 'allowed'
    commandSystemReadOnly = $values['B.system32_read'] -eq 'allowed' -and $values['B.system32_write'] -eq 'denied'
    commandRunsRealComspec = $values['B.comspec'] -match 'System32\\cmd\.exe$' -and $values['B.nested_cmd'] -match 'ok'
    workspaceSharedBothWays = $values['B.reads_from_a'] -eq 'written-by-a' -and $values['B.overwrite_from_a'] -eq 'ok' -and $values['A.reads_from_b'] -eq 'written-by-b' -and $values['A.overwrite_from_b'] -eq 'ok' -and $values['A.delete_b_file'] -eq 'ok'
    exitCodesPropagate = $values['A.bash_exit'] -eq '37' -and $values['A.cmd_exit'] -eq '23' -and $values['A.pwsh_exit'] -eq '29'
    powershellArgumentValidationKept = $values['A.pwsh_bad_args_exit'] -eq '126' -and $values['A.pwsh_bad_args_output'] -match 'GENT-CONTAINED-SHELL-ERROR contained-shell-unsupported-argument'
    consoleStdioRelayed = $values['A.conpty_exit'] -eq '41' -and $values['A.conpty_output_relayed'] -eq 'yes'
    killingShimKillsCommand = $values['A.sleeper_started'] -eq 'yes' -and $sleeperDead -eq $true
    backgroundSurvivesShellThenSettles = $values['A.background_shell_exit'] -eq '0' -and $backgroundAliveAfterShell -eq $true -and $backgroundGoneAfterHelper
    helperSettledBothJobs = -not $timedOut -and $helperExit -eq 0 -and $survivors.Count -eq 0
}
$summary = [ordered]@{ root=$root; helperSha256=(Hash (Join-Path $runtime 'helper.exe')); shimSha256=(Hash $launcher); helperExit=$helperExit; stderr=$errTask.Result.Trim(); controls=$controls; checks=$checks; report=$report; passed=@($checks.Values | Where-Object { -not $_ }).Count -eq 0 }
$summary | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $root 'summary.json')
$summary | ConvertTo-Json -Depth 5
if (-not $summary.passed) { throw 'Two-environment containment fixture failed.' }
