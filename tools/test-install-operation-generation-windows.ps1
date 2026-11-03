$ErrorActionPreference = "Stop"
$Target = "x86_64-pc-windows-msvc"
. (Join-Path $PSScriptRoot "install_archive_validation.ps1")
function Fail([string]$Message) { throw $Message }
function Assert-MutationLease { }
function Get-RuntimeTreeDigest([string]$Path) { return $script:Digest }
function Read-SelectedRelease([string]$Root) { return $script:Current }
function Write-CurrentPointer([string]$Root, [string]$Release) { $script:Current = $Release }
$script:Digest = 'a' * 64
$root = Join-Path ([IO.Path]::GetTempPath()) ("gent-operation-reader-" + [Guid]::NewGuid().ToString('N'))
foreach ($size in @(8, 64)) {
    $previous = "v1.2.3+$($script:Digest.Substring(0, 16)).$('b' * $size)-$Target"
    $script:Current = $previous
    $selected = Publish-ImmutableRelease $root (Join-Path $root "uncreated-source") $script:Digest "v1.2.3"
    if ($selected -ne $previous) { throw "Matching retained generation was not reused" }
    if (Test-Path -LiteralPath $root) { throw "Reader test unexpectedly mutated its fixture scope" }
    $script:Current = "v1.2.4+$($script:Digest.Substring(0, 16)).12345678-$Target"
    Restore-ExactSelection $root $previous $script:Digest $script:Current
    if ($script:Current -ne $previous) { throw "Rollback did not preserve exact retained generation" }
    if (Test-Path -LiteralPath $root) { throw "Rollback unnecessarily copied a retained generation" }
}
Write-Output "Retained generation8 and operation64 reuse/rollback checks passed"
