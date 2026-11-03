# Tiny fake-file coverage. Never runs the installer entry point, native binaries or compiler.
param([Parameter(Mandatory = $true)][string]$Work)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$Target = "x86_64-pc-windows-msvc"
$main = Join-Path $PSScriptRoot "install.ps1"
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($main, [ref]$null, [ref]$errors)
if ($errors.Count) { throw "installer source does not parse" }
foreach ($definition in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text))
}
. (Join-Path $PSScriptRoot "install_archive_validation.ps1")
# Do not call the production Add-Type implementation.
function Move-Atomically([string]$Source, [string]$Destination) {
    [IO.File]::WriteAllBytes($Destination, [IO.File]::ReadAllBytes($Source))
    [IO.File]::Delete($Source)
}
function Assert([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function New-FakeTree([string]$Path, [string]$Marker) {
    [IO.Directory]::CreateDirectory($Path) | Out-Null
    foreach ($relative in @("gent.exe", "gentd.exe", "gent-launcher.exe", "runtime/node/bin/node.exe",
        "runtime/node/bin/npm.cmd", "runtime/node/lib/node_modules/npm/bin/npm-cli.js",
        "runtime/claurst/claurst.exe", "runtime/claurst/llama/llama-server.exe",
        "authority/ordinary-authority.json", "authority/root-keys.json")) {
        $file = Join-Path $Path $relative
        [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($file)) | Out-Null
        [IO.File]::WriteAllText($file, $Marker, [Text.UTF8Encoding]::new($false))
    }
}
$root = Join-Path $Work "store"
$data = Join-Path $Work "data"
$firstStage = Join-Path $root ".gent-stage-first"
New-FakeTree $firstStage "first"
$firstDigest = Get-RuntimeTreeDigest $firstStage
$failed = $false
try { Publish-ImmutableRelease $root $firstStage $firstDigest "v1.2.3" | Out-Null } catch { $failed = $true }
Assert $failed "publication without both leases was permitted"
Assert (-not (Test-Path (Join-Path $root "releases"))) "unleased publication wrote releases"
Use-IdleDaemonLock "" {
    Use-InstallLock $root {
        $script:first = Publish-ImmutableRelease $root $firstStage $firstDigest "v1.2.3"
        Write-CurrentPointer $root $script:first
    }
}
Assert ($script:first -match '^v1\.2\.3\+[0-9a-f]{16}\.[0-9a-f]{8}-x86_64-pc-windows-msvc$') "first selection was not qualified"
$first = Join-Path (Join-Path $root "releases") $script:first
Assert ((Get-RuntimeTreeDigest $first) -eq $firstDigest) "first tree changed during publication"
$secondStage = Join-Path $root ".gent-stage-second"
New-FakeTree $secondStage "repaired"
$secondDigest = Get-RuntimeTreeDigest $secondStage
Use-IdleDaemonLock $data {
    Use-InstallLock $root {
        $script:second = Publish-ImmutableRelease $root $secondStage $secondDigest "v1.2.3"
        Write-CurrentPointer $root $script:second
    }
}
Assert ($script:first -ne $script:second) "same-version repair rewrote the first generation"
Assert ((Get-RuntimeTreeDigest $first) -eq $firstDigest) "repair mutated another owner's old tree"
$called = $false
$held = [IO.File]::Open((Join-Path $data "gentd.lock"), "OpenOrCreate", "ReadWrite", "ReadWrite")
try {
    $held.Lock(0, 1)
    $failed = $false
    try { Use-IdleDaemonLock $data { Use-InstallLock $root { $script:called = $true } } } catch { $failed = $true }
    Assert $failed "busy data-directory owner was ignored"
    Assert (-not $called) "mutation action ran while data lease was busy"
} finally { $held.Unlock(0, 1); $held.Dispose() }
$held = [IO.File]::Open((Join-Path $root ".install.lock"), "OpenOrCreate", "ReadWrite", "ReadWrite")
try {
    $held.Lock(0, 1)
    $failed = $false
    try { Use-IdleDaemonLock $data { Use-InstallLock $root { $script:called = $true } } } catch { $failed = $true }
    Assert $failed "busy global store owner was ignored"
    Assert (-not $called) "mutation action ran while store lease was busy"
} finally { $held.Unlock(0, 1); $held.Dispose() }
$thirdStage = Join-Path $root ".gent-stage-third"
New-FakeTree $thirdStage "newer"
$thirdDigest = Get-RuntimeTreeDigest $thirdStage
Use-IdleDaemonLock $data {
    Use-InstallLock $root {
        $script:third = Publish-ImmutableRelease $root $thirdStage $thirdDigest "v2.0.0"
        Write-CurrentPointer $root $script:third
    }
}
$before = @(Get-ChildItem -LiteralPath (Join-Path $root "releases") -Directory).Count
$failed = $false
try {
    Use-IdleDaemonLock $data { Use-InstallLock $root { Assert-NoNewerSelection $root "v1.2.3" | Out-Null } }
} catch { $failed = $true }
Assert $failed "newer selection was not refused"
Assert ((Read-SelectedRelease $root) -eq $script:third) "newer refusal changed selection"
Assert (@(Get-ChildItem -LiteralPath (Join-Path $root "releases") -Directory).Count -eq $before) "newer refusal published files"
Use-IdleDaemonLock $data {
    Use-InstallLock $root { Restore-ExactSelection $root $script:second $secondDigest $script:third }
}
Assert ((Read-SelectedRelease $root) -eq $script:second) "guarded rollback selected wrong generation"
$failed = $false
try {
    Use-IdleDaemonLock $data {
        Use-InstallLock $root { Restore-ExactSelection $root $script:first ("0" * 64) $script:second }
    }
} catch { $failed = $true }
Assert $failed "rollback accepted an unverified prior tree"
Assert ((Read-SelectedRelease $root) -eq $script:second) "failed identity rollback changed selection"
$failed = $false
try {
    Use-IdleDaemonLock $data {
        Use-InstallLock $root { Restore-ExactSelection $root $script:first $firstDigest $script:third }
    }
} catch { $failed = $true }
Assert $failed "rollback overwrote a changed selection"
$hashRoot = Join-Path $Work "hash"
[IO.Directory]::CreateDirectory($hashRoot) | Out-Null
foreach ($name in @("a", [string][char]0xE000, [char]::ConvertFromUtf32(0x1F600))) {
    [IO.File]::WriteAllBytes((Join-Path $hashRoot $name), [Text.Encoding]::UTF8.GetBytes($name))
}
[pscustomobject]@{ checks = 10; treeSha256 = Get-RuntimeTreeDigest $hashRoot; scope = "extracted helper only; mocked pointer replace; fake files; no installer/native/network/compiler" } | ConvertTo-Json -Compress
