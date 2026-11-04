use std::fs;

pub(super) fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(super) fn required_files() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            "gent.exe",
            "gentd.exe",
            "gent-launcher.exe",
            "gent-auto-update.ps1",
            "runtime/node/bin/node.exe",
            "runtime/node/bin/npm.cmd",
            "runtime/node/lib/node_modules/npm/bin/npm-cli.js",
            "runtime/claurst/claurst.exe",
            "runtime/claurst/llama/llama-server.exe",
            "authority/ordinary-authority.json",
            "authority/root-keys.json",
        ]
    }
    #[cfg(not(windows))]
    {
        &[
            "gent",
            "gentd",
            "gent-auto-update.py",
            "runtime/node/bin/node",
            "runtime/node/bin/npm",
            "runtime/node/lib/node_modules/npm/bin/npm-cli.js",
            "runtime/claurst/claurst",
            "runtime/claurst/llama/llama-server",
            "authority/ordinary-authority.json",
            "authority/root-keys.json",
        ]
    }
}
