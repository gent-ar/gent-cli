//! Reuse the reviewed finite PS7 engine/contained launcher contract for Claurst.
use crate::windows_provider_resources::manifest::{self, ProviderResources, ResourceRoot};
use gent_types::SandboxShellRuntime;
use std::path::Path;

const REQUIRED_ENGINE_FILES: [&str; 9] = [
    "pwsh.exe",
    "pwsh.dll",
    "pwsh.deps.json",
    "pwsh.runtimeconfig.json",
    "hostfxr.dll",
    "hostpolicy.dll",
    "coreclr.dll",
    "System.Private.CoreLib.dll",
    "System.Management.Automation.dll",
];
pub(super) fn require_contract(contract: &ProviderResources) -> Result<(), String> {
    let has = |root, path: &str| {
        contract
            .files
            .iter()
            .any(|file| file.root == root && file.path.eq_ignore_ascii_case(path))
    };
    if !REQUIRED_ENGINE_FILES
        .iter()
        .all(|path| has(ResourceRoot::PowerShellRuntime, path))
        || crate::windows_provider_resources::command_closure::require_command_closure(contract)
            .is_err()
        || !has(
            ResourceRoot::OsGlobalizationData,
            "Globalization/Sorting/sortdefault.nls",
        )
    {
        return Err("Claurst requires the complete reviewed PowerShell7 engine, contained launcher and Windows globalization data; repair its runtime bundle".into());
    }
    Ok(())
}
pub(super) fn attach(
    root: &Path,
    contract: &ProviderResources,
    shell: SandboxShellRuntime,
) -> Result<SandboxShellRuntime, String> {
    require_contract(contract)?;
    // The caller has already hashed and retained immutable handles for every
    // manifest file. These two additional image locks bind the native launcher.
    let engine = manifest::resource_path(&root.join("powershell"), "pwsh.exe")?;
    let launcher = manifest::resource_path(&root.join("powershell-launcher"), "pwsh.exe")?;
    let engine = gent_drivers::lock::capture(
        "powershell7",
        &engine,
        "7-portable",
        "managed-powershell-v1",
    )
    .map_err(|_| "Claurst reviewed PowerShell engine unavailable")?;
    let launcher = gent_drivers::lock::capture(
        "powershell7-launcher",
        &launcher,
        "1",
        "managed-powershell-launcher-v1",
    )
    .map_err(|_| "Claurst reviewed PowerShell launcher unavailable")?;
    shell
        .with_powershell(engine)
        .and_then(|shell| shell.with_powershell_launcher(launcher))
        .and_then(SandboxShellRuntime::with_brokered_command_egress)
        .map_err(|_| "Claurst reviewed PowerShell pair rejected".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contract() -> ProviderResources {
        let mut files: Vec<_> = REQUIRED_ENGINE_FILES
            .iter()
            .map(|name| {
                serde_json::json!({
                    "root":"powerShellRuntime","path":name,"sha256":"a".repeat(64)
                })
            })
            .collect();
        for shim in manifest::COMMAND_SHIMS {
            files.push(serde_json::json!({"root":"powerShellLauncher","path":shim,"sha256":"b".repeat(64)}));
        }
        files.push(serde_json::json!({"root":"osGlobalizationData","path":"Globalization/Sorting/sortdefault.nls","sha256":"c".repeat(64)}));
        for path in crate::windows_provider_resources::command_closure::required_paths().unwrap() {
            files
                .push(serde_json::json!({"root":"gitRuntime","path":path,"sha256":"d".repeat(64)}));
        }
        serde_json::from_value(serde_json::json!({"provider":"claurst","platform":"win32-x64",
            "credentialContract":"claurst-daemon-model-v1","files":files,"osLibraries":["ntdll.dll"]})).unwrap()
    }
    #[test]
    fn complete_pair_required_before_any_process_launch() {
        let complete = contract();
        assert!(require_contract(&complete).is_ok());
        for index in 0..complete.files.len() {
            let mut missing = complete.clone();
            missing.files.remove(index);
            assert!(require_contract(&missing).is_err());
        }
    }
}
