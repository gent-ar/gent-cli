# Run or register Gent's externally supervised Windows runtime updater.
[CmdletBinding()]
param(
    [Parameter(Position = 0, Mandatory = $true)]
    [ValidateSet("run", "enable", "disable", "status")][string]$Action,
    [Parameter(Mandatory = $true)][string]$RuntimeRoot,
    [Parameter(Mandatory = $true)][string]$DataDir,
    [ValidateRange(300, 604800)][int]$IntervalSeconds = 21600,
    [ValidateRange(1, 120)][int]$TimeoutSeconds = 30,
    [string]$SchedulerDir,
    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$Repository = "gent-ar/gent-cli"
$TagPattern = '^v([0-9]+)\.([0-9]+)\.([0-9]+)$'

function Fail([string]$Message) { throw "Gent automatic update refused: $Message" }
function Require-PlainFile([string]$Path, [string]$Name) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { Fail "$Name is missing" }
    if ((Get-Item -LiteralPath $Path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { Fail "$Name is unsafe" }
}
function Current-Release {
    $pointer = Join-Path $RuntimeRoot "current.json"
    Require-PlainFile $pointer "current runtime pointer"
    $release = [string]((Get-Content -Raw -LiteralPath $pointer | ConvertFrom-Json).release)
    if ($release -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+-x86_64-pc-windows-msvc$') { Fail "current runtime pointer is invalid" }
    $path = Join-Path (Join-Path $RuntimeRoot "releases") $release
    foreach ($name in @("gent.exe", "gentd.exe")) { Require-PlainFile (Join-Path $path $name) "current $name" }
    return $release
}
function State-Path { Join-Path $RuntimeRoot "auto-update-state.json" }
function Disabled-Marker { Join-Path $RuntimeRoot ".gent-auto-update-disabled" }
function Clear-DisabledMarker {
    $marker = Disabled-Marker
    if (Test-Path -LiteralPath $marker) { Require-PlainFile $marker "automatic-update preference marker"; Remove-Item -LiteralPath $marker -Force }
}
function Write-DisabledMarker { $marker = Disabled-Marker; if (Test-Path -LiteralPath $marker) { Require-PlainFile $marker "automatic-update preference marker" }; [IO.File]::WriteAllText($marker, "disabled`n", [Text.UTF8Encoding]::new($false)) }
function Read-State {
    $path = State-Path
    if (-not (Test-Path -LiteralPath $path)) { return @{ schemaVersion = 1; failureCount = 0; nextEligibleAt = 0 } }
    try { $decoded = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json; $state = @{}; $decoded.psobject.Properties | ForEach-Object { $state[$_.Name] = $_.Value } } catch { Fail "automatic-update state is invalid" }
    if ($state.schemaVersion -ne 1) { Fail "automatic-update state schema is unsupported" }
    return $state
}
function Merge-State([hashtable]$State, [hashtable]$Extra) { $copy = @{}; $State.GetEnumerator() | ForEach-Object { $copy[$_.Key] = $_.Value }; $Extra.GetEnumerator() | ForEach-Object { $copy[$_.Key] = $_.Value }; return $copy }
function Replace-File([string]$Source, [string]$Destination) {
    if (-not ("GentAutoUpdateMove" -as [type])) { Add-Type -TypeDefinition @'
using System; using System.ComponentModel; using System.Runtime.InteropServices;
public static class GentAutoUpdateMove {
 [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern bool MoveFileEx(string a,string b,int flags);
 public static void Replace(string a,string b) { if (!MoveFileEx(a,b,1|8)) throw new Win32Exception(Marshal.GetLastWin32Error()); }
}
'@ }
    [GentAutoUpdateMove]::Replace($Source, $Destination)
}
function Save-State([hashtable]$State) {
    $path = State-Path; $temporary = Join-Path $RuntimeRoot (".auto-update-" + [guid]::NewGuid().ToString("N"))
    [IO.File]::WriteAllText($temporary, ($State | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
    if (Test-Path -LiteralPath $path) { Replace-File $temporary $path } else { [IO.File]::Move($temporary, $path) }
}
function With-UpdateLock([scriptblock]$Work) {
    $path = Join-Path $RuntimeRoot ".auto-update.lock"
    $lock = [IO.File]::Open($path, "OpenOrCreate", "ReadWrite", "Read")
    try { try { $lock.Lock(0, 1) } catch { Fail "another automatic update is already running" }; & $Work } finally { try { $lock.Unlock(0, 1) } catch {}; $lock.Dispose() }
}
function Latest-Tag {
    $url = if ($env:GENT_RELEASE_API_URL) { $env:GENT_RELEASE_API_URL } else { "https://api.github.com/repos/$Repository/releases/latest" }
    $release = Invoke-RestMethod -Uri $url -TimeoutSec $TimeoutSeconds
    $tag = [string]$release.tag_name
    if ($release.draft -or $release.prerelease -or $tag -notmatch $TagPattern) { Fail "release discovery did not return a stable semantic version tag" }
    return $tag
}
function Version-Number([string]$Tag) { return [version]($Tag.TrimStart("v")) }
function Download([string]$Base, [string]$Name, [string]$Directory) {
    $path = Join-Path $Directory $Name
    Invoke-WebRequest -Uri "$Base/$Name" -OutFile $path -TimeoutSec $TimeoutSeconds
    return $path
}
function Install-Latest([string]$Tag) {
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ("gent-auto-update-" + [guid]::NewGuid().ToString("N")); New-Item -ItemType Directory -Path $temporary | Out-Null
    try {
        $base = if ($env:GENT_RELEASE_DOWNLOAD_BASE_URL) { "$($env:GENT_RELEASE_DOWNLOAD_BASE_URL.TrimEnd('/'))/$Tag" } else { "https://github.com/$Repository/releases/download/$Tag" }
        $installer = Download $base "gent-install.ps1" $temporary; $bundle = Download $base "gent-install.ps1.sigstore.json" $temporary
        & cosign verify-blob $installer --bundle $bundle --certificate-identity-regexp "^https://github.com/$Repository/.github/workflows/release.yml@refs/tags/$([regex]::Escape($Tag))`$" --certificate-oidc-issuer "https://token.actions.githubusercontent.com" | Out-Null
        if ($LASTEXITCODE -ne 0) { Fail "signed installer bootstrap verification failed" }
        & powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $installer -Version $Tag -InstallDir $RuntimeRoot -IdleDataDir $DataDir -Force -RequireHealth
        if ($LASTEXITCODE -ne 0) { Fail "signed installer rejected the update" }
    } finally { if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force -Recurse } }
}
function Update-Once {
    Current-Release | Out-Null; $state = Read-State; $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    if (-not $Force -and [int64]$state.nextEligibleAt -gt $now) { return Merge-State $state @{ result = "backoff" } }
    try {
        $tag = Latest-Tag; $current = (Current-Release).Split("-")[0]
        if ((Version-Number $tag) -le (Version-Number $current)) { $next = @{ schemaVersion = 1; failureCount = 0; nextEligibleAt = 0; lastSeenTag = $tag; result = "current" } }
        else { Install-Latest $tag; $next = @{ schemaVersion = 1; failureCount = 0; nextEligibleAt = 0; lastSeenTag = $tag; lastUpdatedAt = $now; result = "updated" } }
    } catch {
        $failures = [Math]::Min(([int]$state.failureCount + 1), 6); $next = Merge-State $state @{ failureCount = $failures; nextEligibleAt = $now + [Math]::Min(21600, 60 * [Math]::Pow(2, $failures)); lastError = $_.Exception.Message; result = "failed" }
    }
    Save-State $next; return $next
}
function Task-Name {
    $hash = [Security.Cryptography.SHA256]::Create()
    try { $digest = $hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($RuntimeRoot)) }
    finally { $hash.Dispose() }
    return "Gent Automatic Update " + ([BitConverter]::ToString($digest).Replace("-", "").Substring(0, 12))
}
function Scheduled-Arguments { "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$PSCommandPath`" run -RuntimeRoot `"$RuntimeRoot`" -DataDir `"$DataDir`" -TimeoutSeconds $TimeoutSeconds" }
function Schedule-Paths { $dir = if ($SchedulerDir) { $SchedulerDir } else { $null }; if ($null -eq $dir) { return $null }; New-Item -ItemType Directory -Force -Path $dir | Out-Null; return @((Join-Path $dir "gent-auto-update.task.ps1"), (Join-Path $dir "gent-auto-update.task.json")) }
function Enable-Update {
    Current-Release | Out-Null; Clear-DisabledMarker; $paths = Schedule-Paths
    if ($null -ne $paths) { [IO.File]::WriteAllText($paths[0], "& powershell $(Scheduled-Arguments)`n", [Text.UTF8Encoding]::new($false)); @{ taskName = Task-Name; intervalSeconds = $IntervalSeconds } | ConvertTo-Json -Compress | Set-Content -NoNewline -Encoding utf8 $paths[1]; return }
    $trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Seconds $IntervalSeconds)
    Register-ScheduledTask -TaskName (Task-Name) -Action (New-ScheduledTaskAction -Execute "powershell.exe" -Argument (Scheduled-Arguments)) -Trigger $trigger -Description "Gent signed paired-runtime updater" -Force | Out-Null
}
function Disable-Update { $paths = Schedule-Paths; if ($null -ne $paths) { Remove-Item -LiteralPath $paths -Force -ErrorAction SilentlyContinue } else { Unregister-ScheduledTask -TaskName (Task-Name) -Confirm:$false -ErrorAction SilentlyContinue }; Write-DisabledMarker }
function Is-Enabled { $paths = Schedule-Paths; if ($null -ne $paths) { return (Test-Path -LiteralPath $paths[0]) -and (Test-Path -LiteralPath $paths[1]) }; return $null -ne (Get-ScheduledTask -TaskName (Task-Name) -ErrorAction SilentlyContinue) }

try {
    switch ($Action) {
        "run" { With-UpdateLock { Update-Once | ConvertTo-Json -Compress } }
        "enable" { Enable-Update }
        "disable" { Disable-Update }
        "status" { $state = Read-State; $state.enabled = Is-Enabled; $state | ConvertTo-Json -Compress }
    }
} catch { Write-Error $_.Exception.Message; exit 1 }
