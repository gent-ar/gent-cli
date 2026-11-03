param([Parameter(Mandatory=$true)][string]$ReceiptDirectory,
      [ValidateRange(15,300)][int]$MaximumSeconds=180)
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run this rule-only fixture window from an administrator PowerShell. Keep all helper and connector probes in the original medium-integrity session.' }
if (Test-Path -LiteralPath $ReceiptDirectory) { throw 'Choose a new receipt directory; existing directories are preserved.' }
New-Item -ItemType Directory -Path $ReceiptDirectory | Out-Null
$receiptRoot = [IO.Path]::GetFullPath($ReceiptDirectory)
$ruleName = 'GentSandboxLanWindow-' + [guid]::NewGuid().ToString('N')
$complete = Join-Path $receiptRoot 'complete'
$created = $false
$report = [ordered]@{ ruleName=$ruleName; protocol='TCP'; localPort=45682; localAddress='192.168.1.195'; remoteAddress='192.168.1.185';
    absentBefore=$false; created=$false; ready=$false; completed=$false; removed=$false }
try {
    if (@(Get-NetIPAddress -IPAddress 192.168.1.195 -ErrorAction Stop).Count -eq 0) { throw 'The approved fixture address is not assigned to this host.' }
    $before = @(Get-NetFirewallRule -PolicyStore ActiveStore -ErrorAction Stop | Where-Object Name -eq $ruleName)
    $report.absentBefore = $before.Count -eq 0
    if (-not $report.absentBefore) { throw 'The unique fixture rule name already exists.' }
    New-NetFirewallRule -Name $ruleName -DisplayName $ruleName -Direction Inbound -Action Allow -Protocol TCP -LocalPort 45682 -LocalAddress 192.168.1.195 -RemoteAddress 192.168.1.185 -Profile Any -ErrorAction Stop | Out-Null
    $created = $true; $report.created = $true
    $deadline = [DateTime]::UtcNow.AddSeconds($MaximumSeconds)
    $ready = [ordered]@{ ruleName=$ruleName; localPort=45682; localAddress='192.168.1.195'; remoteAddress='192.168.1.185';
        deadlineUtc=$deadline.ToString('o'); completionMarker=$complete; elevatedProbesPermitted=$false }
    $ready | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $receiptRoot 'ready.json')
    $report.ready = $true
    Write-Output ('Scoped firewall window ready: ' + (Join-Path $receiptRoot 'ready.json'))
    while ([DateTime]::UtcNow -lt $deadline -and -not (Test-Path -LiteralPath $complete)) { Start-Sleep -Milliseconds 250 }
    $report.completed = Test-Path -LiteralPath $complete
} catch {
    $report['error'] = $_.FullyQualifiedErrorId
    throw
} finally {
    if ($created) { try { Remove-NetFirewallRule -Name $ruleName -ErrorAction Stop } catch { $report['removalError'] = $_.FullyQualifiedErrorId } }
    try { $after = @(Get-NetFirewallRule -PolicyStore ActiveStore -ErrorAction Stop | Where-Object Name -eq $ruleName); $report.removed = $after.Count -eq 0 }
    catch { $report['removalVerificationError'] = $_.FullyQualifiedErrorId }
    $report | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $receiptRoot 'cleanup.json')
    if (-not $report.removed) { throw 'The exact fixture rule still exists; inspect its recorded name.' }
}
