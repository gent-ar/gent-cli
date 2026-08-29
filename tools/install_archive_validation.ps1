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
}
function Assert-ZipMembers([string]$Archive, [string]$ReleaseVersion) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $root = "gent-$ReleaseVersion-$Target"
    $required = @("$root/gent.exe", "$root/gentd.exe", "$root/gent-launcher.exe", "$root/runtime/node/bin/node.exe", "$root/runtime/node/bin/npm.cmd", "$root/runtime/node/lib/node_modules/npm/bin/npm-cli.js", "$root/runtime/claurst/claurst.exe", "$root/runtime/claurst/llama/llama-server.exe")
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
    foreach ($name in @("runtime/node/bin/node.exe", "runtime/node/bin/npm.cmd", "runtime/node/lib/node_modules/npm/bin/npm-cli.js", "runtime/claurst/claurst.exe", "runtime/claurst/llama/llama-server.exe")) { Assert-PlainFile (Join-Path $ReleasePath $name) "packaged runtime $name" }
    Get-ChildItem -LiteralPath (Join-Path $ReleasePath "runtime") -Force -Recurse | ForEach-Object { if ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) { Fail "packaged runtime cannot contain a reparse point" } }
}
