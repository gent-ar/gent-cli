param([Parameter(Mandatory=$true)][string]$Helper, [Parameter(Mandatory=$true)][string]$Probe, [switch]$UnsafeReadonlyAcl, [switch]$UnsafeHardLink, [switch]$Gateway, [int]$GatewayPort=45679,
      [string]$ArtifactDirectory = (Join-Path $PSScriptRoot 'artifacts'))
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $ArtifactDirectory | Out-Null
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('gent-sandbox-fixture-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
$workspace = Join-Path $fixtureRoot 'workspace'
$runtime = Join-Path $fixtureRoot 'runtime'
$outside = Join-Path $fixtureRoot 'outside'
$readonly = Join-Path $workspace 'readonly'
$writable = Join-Path $workspace 'writable'
$state = Join-Path $fixtureRoot 'private-state'
foreach ($dir in @($workspace, $runtime, $outside, $readonly, $writable, $state)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$helperPath = Join-Path $runtime 'gent-windows-sandbox.exe'
$probePath = Join-Path $runtime 'gent-windows-sandbox-probe.exe'
Copy-Item -LiteralPath $Helper -Destination $helperPath
Copy-Item -LiteralPath $Probe -Destination $probePath
[IO.File]::WriteAllText((Join-Path $readonly 'input.txt'), 'allowed')
[IO.File]::WriteAllText((Join-Path $outside 'secret.txt'), 'fixture-only-secret')
$broadAclPaths = @($outside, (Join-Path $outside 'secret.txt'))
if ($UnsafeReadonlyAcl) { $broadAclPaths += @($readonly, (Join-Path $readonly 'input.txt')) }
foreach ($path in $broadAclPaths) {
    $acl = Get-Acl -LiteralPath $path
    foreach ($sidText in @('S-1-1-0', 'S-1-15-2-1', 'S-1-15-2-2')) {
        $sid = New-Object Security.Principal.SecurityIdentifier($sidText)
        $rule = New-Object Security.AccessControl.FileSystemAccessRule($sid, 'FullControl', 'Allow')
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $path -AclObject $acl
}
$escape = Join-Path $writable 'escape'
New-Item -ItemType Junction -Path $escape -Target $outside | Out-Null
if ($UnsafeHardLink) { New-Item -ItemType HardLink -Path (Join-Path $writable 'unsafe-hardlink.txt') -Target (Join-Path $outside 'secret.txt') | Out-Null }
$originalPaths = @($fixtureRoot, $workspace, $runtime, $outside, $readonly, $writable, $helperPath, $probePath,
    (Join-Path $readonly 'input.txt'), (Join-Path $outside 'secret.txt'))
$before = @{}; foreach ($path in $originalPaths) { $before[$path] = (Get-Acl -LiteralPath $path).Sddl }
function Quote-Argument([string]$value) {
    return '"' + ([regex]::Replace([regex]::Replace($value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1')) + '"'
}
$digest = (Get-FileHash -LiteralPath $probePath -Algorithm SHA256).Hash.ToLowerInvariant()
$profile = 'a' * 64
$reportPath = Join-Path $writable 'report.json'
$providerArgs = @((Join-Path $readonly 'input.txt'), (Join-Path $writable 'output.txt'),
    (Join-Path $outside 'secret.txt'), (Join-Path $outside 'forbidden.txt'), (Join-Path $escape 'secret.txt'),
    (Join-Path $readonly 'forbidden.txt'), $reportPath, $probePath)
$argsList = @('run', '--workspace', $workspace, '--read', $workspace, '--read', $runtime, '--write', $writable,
    '--read', (Join-Path $env:SystemRoot 'System32/mswsock.dll'),
    '--exe', $probePath, '--sha256', $digest, '--profile', $profile, '--run', 'fixture-run',
    '--processes', '4', '--memory', '268435456', '--cpu-ms', '10000')
if ($Gateway) {
    $peerNonce = [guid]::NewGuid().ToString('N')
    $publicAddress = [Net.Dns]::GetHostAddresses('www.microsoft.com') | Where-Object { $_.AddressFamily -eq [Net.Sockets.AddressFamily]::InterNetwork } | Select-Object -First 1
    if ($null -eq $publicAddress) { throw 'Public network control has no IPv4 address' }
    $publicControl = New-Object Net.Sockets.TcpClient
    try { if (-not $publicControl.ConnectAsync($publicAddress, 443).Wait(5000)) { throw 'Public network control unavailable' } } finally { $publicControl.Close() }
    $argsList += @('--allow-loopback-port', "$GatewayPort", '--gateway-api', 'claude', '--gateway-url', "http://127.0.0.1:$GatewayPort", '--gateway-token', ('b' * 64))
    $argsList += @('--write', $state, '--state-root', $state, '--peer-nonce', $peerNonce)
    $providerArgs += @("$GatewayPort", '45680', (Join-Path $state "GentGateway-$peerNonce"), $publicAddress.ToString())
}
$argsList += @('--') + $providerArgs
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = $probePath
$wrapperArguments = @('host-inherited-handle', (Join-Path $outside 'secret.txt'), $helperPath) + $argsList
$start.Arguments = ($wrapperArguments | ForEach-Object { Quote-Argument $_ }) -join ' '
$start.UseShellExecute = $false; $start.CreateNoWindow = $true
$start.RedirectStandardInput = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
$start.EnvironmentVariables['GENT_FIXTURE_AMBIENT_SECRET'] = 'fixture-only-never-inherit'
$process = New-Object Diagnostics.Process; $process.StartInfo = $start
$stdout = New-Object IO.MemoryStream; $stderr = New-Object IO.MemoryStream
$listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 45679)
$listener.Start()
$otherListener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 45680)
$otherListener.Start()
$ipv6Listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::IPv6Loopback, 45679)
$ipv6Listener.Start()
$control = New-Object Net.Sockets.TcpClient
$control.Connect([Net.IPAddress]::Loopback, 45679)
$acceptedControl = $listener.AcceptTcpClient()
$control.Close()
$acceptedControl.Close()
$inboundControlListener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 45682)
$inboundControlListener.Start()
$inboundControl = New-Object Net.Sockets.TcpClient
$inboundControl.Connect([Net.IPAddress]::Loopback, 45682)
$inboundAccepted = $inboundControlListener.AcceptTcpClient(); $inboundControl.Close(); $inboundAccepted.Close(); $inboundControlListener.Stop()
$acceptTask = $listener.AcceptTcpClientAsync()
$started = $process.Start()
$outTask = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
$errTask = $process.StandardError.BaseStream.CopyToAsync($stderr)
$payload = [byte[]]@(0, 10, 13, 27, 127, 195, 169, 255)
$process.StandardInput.BaseStream.Write($payload, 0, $payload.Length)
$process.StandardInput.Close()
if ($Gateway) {
    $gatewayDeadline = [DateTime]::UtcNow.AddSeconds(15)
    while (-not $acceptTask.IsCompleted -and -not $process.HasExited -and [DateTime]::UtcNow -lt $gatewayDeadline) { Start-Sleep -Milliseconds 25 }
    if ($acceptTask.IsCompleted -and -not $acceptTask.IsFaulted) {
        $client = $acceptTask.Result; $stream = $client.GetStream(); $stream.ReadTimeout = 5000
        $header = ''; $single = New-Object byte[] 1
        while (-not $header.EndsWith("`r`n`r`n") -and $header.Length -lt 32768 -and $stream.Read($single, 0, 1) -eq 1) { $header += [char]$single[0] }
        $gatewayHostRewritten = $header.Contains("`r`nHost: 127.0.0.1:$GatewayPort`r`n") -and $header.Contains("`r`nConnection: close`r`n")
        $response = [Text.Encoding]::ASCII.GetBytes("HTTP/1.1 200 OK`r`nContent-Length: 2`r`nConnection: close`r`n`r`nOK")
        $stream.Write($response, 0, $response.Length); $client.Close()
    }
    $ready = Join-Path $state 'tmp/listener-ready'; $readyDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $ready) -and -not $process.HasExited -and [DateTime]::UtcNow -lt $readyDeadline) { Start-Sleep -Milliseconds 25 }
    if (Test-Path -LiteralPath $ready) {
        $inboundStatus = [IO.File]::ReadAllText($ready).Trim()
        if ($inboundStatus -eq 'bound') {
            $incoming = New-Object Net.Sockets.TcpClient
            $incomingAttempt = $incoming.BeginConnect([Net.IPAddress]::Loopback, 45682, $null, $null)
            $inboundDenied = -not $incomingAttempt.AsyncWaitHandle.WaitOne(1000)
            if (-not $inboundDenied) { try { $incoming.EndConnect($incomingAttempt); $inboundDenied = -not $incoming.Connected } catch { $inboundDenied = $true } }
            $incoming.Close()
        } else { $inboundDenied = $inboundStatus -eq 'denied:10013' }
        [IO.File]::WriteAllText((Join-Path $state 'tmp/listener-checked'), 'checked')
    }
}
if (-not $process.WaitForExit(30000)) { $process.Kill(); throw 'Sandbox fixture timed out; helper killed and its job closes.' }
$listener.Stop()
$otherListener.Stop()
$ipv6Listener.Stop()
$outTask.Wait(); $errTask.Wait()
$outBytes = $stdout.ToArray(); $errBytes = $stderr.ToArray()
[IO.File]::WriteAllBytes((Join-Path $ArtifactDirectory 'stdout.bin'), $outBytes)
[IO.File]::WriteAllBytes((Join-Path $ArtifactDirectory 'stderr.bin'), $errBytes)
$aclUnchanged = $true
foreach ($path in $originalPaths) { if ((Get-Acl -LiteralPath $path).Sddl -cne $before[$path]) { $aclUnchanged = $false } }
$lineEnd = [Array]::IndexOf($outBytes, [byte]10)
$header = if ($lineEnd -ge 0) { [Text.Encoding]::ASCII.GetString($outBytes, 0, $lineEnd) } else { '' }
$expectedOutput = [byte[]]($payload + $payload)
$actualOutput = [byte[]]@()
if ($lineEnd -ge 0 -and $lineEnd + 1 -lt $outBytes.Length) { $actualOutput = [byte[]]$outBytes[($lineEnd + 1)..($outBytes.Length - 1)] }
$byteExact = [Convert]::ToBase64String([byte[]]$actualOutput) -ceq [Convert]::ToBase64String($expectedOutput)
$stderrExact = [Convert]::ToBase64String($errBytes) -ceq [Convert]::ToBase64String($payload)
$report = if (Test-Path -LiteralPath $reportPath) { Get-Content -Raw -LiteralPath $reportPath | ConvertFrom-Json } else { $null }
$summary = [ordered]@{ fixtureRoot=$fixtureRoot; helperSha256=(Get-FileHash -LiteralPath $Helper).Hash.ToLowerInvariant(); probeSha256=$digest; exitCode=$process.ExitCode; header=$header; aclUnchanged=$aclUnchanged;
    stdoutByteExact=$byteExact; stderrByteExact=$stderrExact; probes=$report;
    gatewayHostRewritten=$gatewayHostRewritten;
    inboundStatus=$inboundStatus; nonpeerHostInboundDenied=$inboundDenied;
    passed=($process.ExitCode -eq 0 -and $aclUnchanged -and $byteExact -and $stderrExact -and $header -match '^GENT-SANDBOX/1 [0-9]+:[0-9]+ a{64} fixture-run( peer=GentGateway-[a-f0-9]{32},[0-9]+,[0-9]+,[0-9]+)?$' -and (-not $Gateway -or ($gatewayHostRewritten -and $inboundDenied))) }
if ($UnsafeReadonlyAcl -or $UnsafeHardLink) {
    $expectedError = if ($UnsafeReadonlyAcl) { 'readonly-ambient-write-authority' } else { 'filesystem-hardlink-escape' }
    $summary['expectedPrelaunchRefusal'] = $expectedError
    $summary.passed = $process.ExitCode -eq 126 -and $aclUnchanged -and $outBytes.Length -eq 0 -and [Text.Encoding]::ASCII.GetString($errBytes).Contains($expectedError)
}
$summary | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $ArtifactDirectory 'summary.json')
$summary | ConvertTo-Json -Depth 5
if (-not $summary.passed) { throw 'Sandbox enforcement fixture did not pass. See summary and stderr artifacts; do not enable broad modes.' }
