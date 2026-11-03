function Assert-Archive([string]$Archive, [string]$ManifestPath, [string]$ChecksumPath, [string]$ReleaseVersion) {
    $manifest = Get-Content -Raw -LiteralPath $ManifestPath | ConvertFrom-Json
    $digest = (Get-FileHash -Algorithm SHA256 -LiteralPath $Archive).Hash.ToLowerInvariant()
    $expectedName = Split-Path $Archive -Leaf
    $expectedChecksum = "$digest  $expectedName"
    if ($manifest.schemaVersion -ne 1 -or $manifest.version -ne $ReleaseVersion -or $manifest.target -ne $Target -or $manifest.archive.name -ne $expectedName -or $manifest.archive.sha256 -ne $digest -or $manifest.archive.size -ne (Get-Item -LiteralPath $Archive).Length -or (Get-Content -Raw -LiteralPath $ChecksumPath).Trim() -ne $expectedChecksum) { Fail "release archive verification failed" }
    $binaries = @($manifest.binaries | Sort-Object)
    $expectedBinaries = @("gent.exe", "gentd.exe", "gent-launcher.exe" | Sort-Object)
    if (($binaries -join ',') -ne ($expectedBinaries -join ',')) { Fail "release manifest has invalid binaries" }
    if ((@($manifest.runtimes) -join ',') -ne "runtime/node,runtime/claurst") { Fail "release manifest has invalid runtimes" }
    $requiredCapabilities = @("agent-chat-conversations-v1", "agent-chat-intents-v1", "agent-chat-transcript-import-v1", "agent-chat-projection-v1", "agent-chat-transcript-v1", "agent-chat-turn-follow-v1", "agent-chat-permissions-v1", "attachments-v1", "local-models-v1", "workspace-git-v1", "agent-chat-conversation-config-v1", "agent-chat-checkpoint-v1", "agent-chat-side-question-v1", "provider-auth-v1", "provider-readiness-v2", "permission-policy-v2", "prompt-provider-provision-v1")
    foreach ($capability in $requiredCapabilities) { if ($capability -notin @($manifest.capabilities)) { Fail "release manifest has incomplete capabilities" } }
}
function Assert-ZipMembers([string]$Archive, [string]$ReleaseVersion) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $root = "gent-$ReleaseVersion-$Target"
    $required = @("$root/gent.exe", "$root/gentd.exe", "$root/gent-launcher.exe", "$root/runtime/node/bin/node.exe", "$root/runtime/node/bin/npm.cmd", "$root/runtime/node/lib/node_modules/npm/bin/npm-cli.js", "$root/runtime/claurst/claurst.exe", "$root/runtime/claurst/llama/llama-server.exe", "$root/authority/ordinary-authority.json", "$root/authority/root-keys.json")
    $zip = [System.IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $members = @($zip.Entries | ForEach-Object { $_.FullName } | Sort-Object)
        $unexpected = @($members | Where-Object { $_ -notin $required -and -not $_.StartsWith("$root/runtime/node/") -and -not $_.StartsWith("$root/runtime/claurst/llama/") })
        $missing = @($required | Where-Object { $_ -notin $members })
        if ($unexpected.Count -ne 0 -or $missing.Count -ne 0) { Fail "release archive contains unsafe or unexpected paths" }
        foreach ($entry in $zip.Entries) {
            if ($entry.FullName.Contains('..') -or $entry.FullName.StartsWith('/') -or $entry.FullName -match '^[A-Za-z]:' -or $entry.Name.Length -eq 0) { Fail "release archive contains an unsafe path" }
        }
    } finally { $zip.Dispose() }
}
function Assert-PackagedRuntime([string]$ReleasePath) {
    foreach ($name in @("runtime/node/bin/node.exe", "runtime/node/bin/npm.cmd", "runtime/node/lib/node_modules/npm/bin/npm-cli.js", "runtime/claurst/claurst.exe", "runtime/claurst/llama/llama-server.exe", "authority/ordinary-authority.json", "authority/root-keys.json")) { Assert-PlainFile (Join-Path $ReleasePath $name) "packaged runtime $name" }
    Get-ChildItem -LiteralPath (Join-Path $ReleasePath "runtime") -Force -Recurse | ForEach-Object { if ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) { Fail "packaged runtime cannot contain a reparse point" } }
}
function Read-SelectedRelease([string]$Root) {
    $pointer = Join-Path $Root "current.json"
    if (-not (Test-Path -LiteralPath $pointer)) { return $null }
    Assert-PlainFile $pointer "current runtime pointer"
    $value = Get-Content -Raw -LiteralPath $pointer | ConvertFrom-Json
    $names = @($value.PSObject.Properties.Name)
    if ($names.Count -ne 1 -or $names[0] -ne "release") { Fail "current runtime pointer schema is invalid" }
    $release = [string]$value.release
    Assert-ReleaseName $release
    return $release
}
function Release-Version([string]$Name) {
    if ($Name -notmatch '^v([0-9]+\.[0-9]+\.[0-9]+)(?:[+-]|$)') { Fail "runtime version is invalid" }
    return [Version]$Matches[1]
}
function Assert-NoNewerSelection([string]$Root, [string]$Requested) {
    $current = Read-SelectedRelease $Root
    if ($current -and (Release-Version $current) -gt (Release-Version $Requested)) { Fail "a newer runtime is selected; no runtime files were published" }
    return $current
}
function Get-RuntimeTreeDigest([string]$Root) {
    Assert-PlainDirectory $Root "runtime tree"
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push([IO.Path]::GetFullPath($Root))
    $files = [Collections.Generic.List[object]]::new()
    [UInt64]$bytes = 0
    while ($pending.Count) {
        foreach ($path in [IO.Directory]::GetFileSystemEntries($pending.Pop())) {
            $item = Get-Item -LiteralPath $path -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { Fail "runtime tree contains a reparse point" }
            if ($item.PSIsContainer) { $pending.Push($path); continue }
            $bytes += $item.Length
            $relative = $path.Substring($prefix.Length).Replace('\', '/')
            $files.Add([pscustomobject]@{ Path = $path; Bytes = [Text.Encoding]::UTF8.GetBytes($relative) })
            if ($files.Count -gt 20000 -or $bytes -gt 8589934592) { Fail "runtime tree exceeds its file or byte limit" }
        }
    }
    $files.Sort([Comparison[object]]{
        param($left, $right)
        for ($i = 0; $i -lt [Math]::Min($left.Bytes.Length, $right.Bytes.Length); $i++) {
            if ($left.Bytes[$i] -lt $right.Bytes[$i]) { return -1 }
            if ($left.Bytes[$i] -gt $right.Bytes[$i]) { return 1 }
        }
        return $left.Bytes.Length.CompareTo($right.Bytes.Length)
    })
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        $start = [Text.Encoding]::ASCII.GetBytes("gent-runtime-tree-v1" + [char]0)
        $hash.TransformBlock($start, 0, $start.Length, $start, 0) | Out-Null
        foreach ($file in $files) {
            $length = [BitConverter]::GetBytes([UInt64]$file.Bytes.Length)
            if ([BitConverter]::IsLittleEndian) { [Array]::Reverse($length) }
            foreach ($part in @($length, $file.Bytes)) { $hash.TransformBlock($part, 0, $part.Length, $part, 0) | Out-Null }
            $content = [Security.Cryptography.SHA256]::Create()
            $stream = [IO.File]::Open($file.Path, "Open", "Read", "Read")
            try { $digest = $content.ComputeHash($stream) } finally { $stream.Dispose(); $content.Dispose() }
            $hash.TransformBlock($digest, 0, $digest.Length, $digest, 0) | Out-Null
            $mode = [byte[]]@(0, 0, 0, 0)
            $hash.TransformBlock($mode, 0, 4, $mode, 0) | Out-Null
        }
        $hash.TransformFinalBlock([byte[]]@(), 0, 0) | Out-Null
        return [BitConverter]::ToString($hash.Hash).Replace("-", "").ToLowerInvariant()
    } finally { $hash.Dispose() }
}
$script:GentDataLeaseHeld = $false
$script:GentStoreLeaseHeld = $false
$script:GentIdleDataMissing = $false
function Assert-MutationLease {
    if (-not $script:GentDataLeaseHeld -or -not $script:GentStoreLeaseHeld) { Fail "runtime mutations require both host and store leases" }
}
function Use-InstallLock([string]$Root, [scriptblock]$Action) {
    if (-not $script:GentDataLeaseHeld) { Fail "the exact data-directory lease must be acquired first" }
    if ($script:GentIdleDataMissing -and (Read-SelectedRelease $Root)) { Fail "updating an existing runtime requires explicit -IdleDataDir" }
    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    Assert-PlainDirectory $Root "runtime root"
    $path = Join-Path $Root ".install.lock"
    if (Test-Path -LiteralPath $path) { Assert-PlainFile $path "installer lock" }
    $lock = [IO.File]::Open($path, "OpenOrCreate", "ReadWrite", "ReadWrite")
    try {
        try { $lock.Lock(0, 1) } catch { Fail "another Gent install is already in progress" }
        try { $script:GentStoreLeaseHeld = $true; & $Action } finally { $script:GentStoreLeaseHeld = $false; $lock.Unlock(0, 1) }
    } finally { $lock.Dispose() }
}
function Use-IdleDaemonLock([string]$DataDir, [scriptblock]$Action) {
    if (-not $DataDir) {
        try { $script:GentDataLeaseHeld = $true; $script:GentIdleDataMissing = $true; & $Action }
        finally { $script:GentDataLeaseHeld = $false; $script:GentIdleDataMissing = $false }
        return
    }
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    Assert-PlainDirectory $DataDir "daemon data directory"
    $path = Join-Path $DataDir "gentd.lock"
    if (Test-Path -LiteralPath $path) { Assert-PlainFile $path "daemon lock" }
    $lock = [IO.File]::Open($path, "OpenOrCreate", "ReadWrite", "ReadWrite")
    try {
        try { $lock.Lock(0, 1) } catch { Fail "gentd is running for $DataDir; stop it before updating" }
        try { $script:GentDataLeaseHeld = $true; & $Action } finally { $script:GentDataLeaseHeld = $false; $lock.Unlock(0, 1) }
    } finally { $lock.Dispose() }
}
function Publish-ImmutableRelease([string]$Root, [string]$Source, [string]$Digest, [string]$Requested) {
    Assert-MutationLease
    if ((Get-RuntimeTreeDigest $Source) -ne $Digest) { Fail "prepared source identity changed before publication" }
    $current = Read-SelectedRelease $Root
    if ($current -and $current -match '\+[0-9a-f]{16}\.(?:[0-9a-f]{8}|[0-9a-f]{64})-' -and (Release-Version $current) -eq (Release-Version $Requested)) {
        $selected = Join-Path (Join-Path $Root "releases") $current
        try { if ((Get-RuntimeTreeDigest $selected) -eq $Digest) { return $current } } catch { }
    }
    $name = "$Requested+$($Digest.Substring(0, 16)).$([Guid]::NewGuid().ToString('N').Substring(0, 8))-$Target"
    $releases = Join-Path $Root "releases"
    New-Item -ItemType Directory -Force -Path $releases | Out-Null
    Assert-PlainDirectory $releases "runtime releases directory"
    $destination = Join-Path $releases $name
    if (Test-Path -LiteralPath $destination) { Fail "immutable generation already exists" }
    [IO.Directory]::Move($Source, $destination)
    return $name
}
function Restore-ExactSelection([string]$Root, [string]$Previous, [string]$PriorDigest, [string]$Changed) {
    Assert-MutationLease
    if ((Read-SelectedRelease $Root) -ne $Changed) { Fail "selection changed before rollback; no newer selection was overwritten" }
    if (-not $Previous) {
        $pointer = Join-Path $Root "current.json"
        Assert-PlainFile $pointer "current runtime pointer"
        Remove-Item -LiteralPath $pointer
        return
    }
    $release = Join-Path (Join-Path $Root "releases") $Previous
    if (-not $PriorDigest -or (Get-RuntimeTreeDigest $release) -ne $PriorDigest) { Fail "prior runtime identity changed; rollback was refused" }
    $restored = $Previous
    if ($Previous -notmatch '\+[0-9a-f]{16}\.(?:[0-9a-f]{8}|[0-9a-f]{64})-') {
        $stage = Join-Path $Root (".gent-stage-" + [Guid]::NewGuid().ToString("N"))
        [IO.Directory]::CreateDirectory($stage) | Out-Null
        try {
            Get-ChildItem -LiteralPath $release -Force | Copy-Item -Destination $stage -Recurse
            if ((Get-RuntimeTreeDigest $stage) -ne $PriorDigest) { Fail "prior runtime copy changed during rollback" }
            $restored = Publish-ImmutableRelease $Root $stage $PriorDigest ("v" + (Release-Version $Previous).ToString())
        } finally { Remove-PrivateDirectory $stage $Root ".gent-stage-" }
    }
    Write-CurrentPointer $Root $restored
    $path = Join-Path (Join-Path $Root "releases") $restored
    if ((Read-SelectedRelease $Root) -ne $restored -or (Get-RuntimeTreeDigest $path) -ne $PriorDigest) { Fail "rollback selection could not be verified" }
}
