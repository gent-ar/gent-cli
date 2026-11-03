param([Parameter(Mandatory=$true)][string]$Helper, [Parameter(Mandatory=$true)][string]$Probe,
      [Parameter(Mandatory=$true)][string]$ArtifactDirectory)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $ArtifactDirectory | Out-Null
$root = Join-Path ([IO.Path]::GetTempPath()) ('gent-shell-fixture-' + [guid]::NewGuid().ToString('N'))
$runtime = Join-Path $root 'runtime'; $workspace = Join-Path $root 'workspace'; $outside = Join-Path $root 'outside'
foreach ($directory in @($runtime,$workspace,$outside)) { New-Item -ItemType Directory -Path $directory | Out-Null }
$hostImage = Join-Path $runtime 'helper.exe'; $providerImage = Join-Path $runtime 'probe.exe'; $shellImage = Join-Path $runtime 'shell.exe'
Copy-Item -LiteralPath $Helper -Destination $hostImage
Copy-Item -LiteralPath $Probe -Destination $providerImage
Copy-Item -LiteralPath $Probe -Destination $shellImage
$secret = Join-Path $outside 'fixture-secret'; [IO.File]::WriteAllText($secret,'fixture-only')
$providerHash = (Get-FileHash -LiteralPath $providerImage -Algorithm SHA256).Hash.ToLowerInvariant()
$results = @()
foreach ($case in @('valid','wrong-digest','missing-read-grant','writable-shell')) {
    $arguments = @('run','--workspace',$workspace,'--read',$providerImage,'--read',$hostImage,'--write',$workspace,
        '--exe',$providerImage,'--sha256',$providerHash,'--profile',('a'*64),'--run','shell-fixture','--processes','3',
        '--memory','134217728','--cpu-ms','10000','--shell-exe',$shellImage,'--shell-sha256',$(if($case -eq 'wrong-digest'){'b'*64}else{$providerHash}))
    if ($case -ne 'missing-read-grant') { $arguments += @('--read',$shellImage) }
    if ($case -eq 'valid') { $arguments += @('--read',$runtime) }
    if ($case -eq 'writable-shell') { $arguments += @('--write',$shellImage) }
    $arguments += @('--','shell-contract',$secret)
    $start = New-Object Diagnostics.ProcessStartInfo; $start.FileName = $hostImage
    $start.Arguments = ($arguments | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $start.UseShellExecute = $false; $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
    $start.EnvironmentVariables['GENT_FIXTURE_AMBIENT_SECRET'] = 'fixture-only'
    $process = New-Object Diagnostics.Process; $process.StartInfo = $start
    $null = $process.Start(); $process.StandardInput.Close()
    $outTask = $process.StandardOutput.ReadToEndAsync(); $errTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(10000)) { $process.Kill(); $process.WaitForExit(); throw 'Owned shell fixture exceeded timeout.' }
    $stdout = $outTask.Result; $stderr = $errTask.Result
    $expectedLabel = switch ($case) { 'wrong-digest' {'shell-digest'} 'missing-read-grant' {'shell-read-grant'} 'writable-shell' {'shell-writable-grant'} default {''} }
    $passed = if ($case -eq 'valid') { $process.ExitCode -eq 0 -and $stdout.StartsWith('GENT-SANDBOX/1 ') -and -not $stderr }
        else { $process.ExitCode -eq 126 -and -not $stdout -and $stderr.Contains($expectedLabel + ':') }
    $results += [ordered]@{case=$case;exitCode=$process.ExitCode;passed=$passed;stderr=$stderr.Trim()}
    $process.Dispose()
}
$summary = [ordered]@{root=$root;helperSha256=(Get-FileHash -LiteralPath $Helper -Algorithm SHA256).Hash.ToLowerInvariant();cases=$results;passed=@($results | Where-Object {-not $_.passed}).Count -eq 0}
$summary | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $ArtifactDirectory 'summary.json')
$summary | ConvertTo-Json -Depth 5
if (-not $summary.passed) { throw 'Native shell contract fixture failed.' }
