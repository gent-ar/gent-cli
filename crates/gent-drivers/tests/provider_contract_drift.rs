use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gent_drivers::PublicProvider;
use gent_drivers::codex_session::{CodexSessionConfig, CodexTurnOptions};
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_drivers::public_protocol::{PublicWireFact, normalize_public_frame};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, NormalizedProviderEvent,
};
use serde_json::{Value, json};

const CODEX_METHODS_OUTSIDE_SCHEMA: &[(&str, &str)] = &[(
    "currentTime/read",
    "present in the pinned native codex binary but omitted from its generated app-server schema",
)];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
    )
    .unwrap()
}

fn snapshot(provider: &str, file: &str) -> Value {
    let contracts = workspace().join("fixtures/provider-contracts");
    let pins = read_json(&contracts.join("pins.json"));
    let version = pins["providers"][provider]["version"].as_str().unwrap();
    read_json(&contracts.join(provider).join(version).join(file))
}

fn production_source(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    text.split("#[cfg(test)]").next().unwrap().to_owned()
}

fn sources(directory: &Path, include: &dyn Fn(&Path) -> bool) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(sources(&path, include));
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && include(&path)
            && !path.to_string_lossy().ends_with("_tests.rs")
        {
            found.push((path.clone(), production_source(&path)));
        }
    }
    found.sort();
    found
}

fn literals(source: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\'' {
            while let Some(next) = characters.next() {
                if next == '\\' {
                    characters.next();
                } else if next == '\'' || !next.is_ascii_graphic() {
                    break;
                }
            }
            continue;
        }
        if character != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some(next) = characters.next() {
            match next {
                '\\' => {
                    characters.next();
                }
                '"' => break,
                other => value.push(other),
            }
        }
        values.push(value);
    }
    values
}

fn is_codex_source(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains("/src/codex") || text.contains("/public_protocol/codex")
}

fn codex_methods() -> Value {
    snapshot("codex", "protocol.json")["methods"].clone()
}

fn looks_like_method(value: &str) -> bool {
    let mut parts = value.split('/');
    let first = parts.next().unwrap_or_default();
    !first.is_empty()
        && first
            .chars()
            .all(|character| character.is_ascii_alphabetic())
        && value.contains('/')
        && value.split('/').skip(1).all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

fn codex_config() -> CodexSessionConfig {
    CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: None,
        turn_options: CodexTurnOptions::from_selection(
            &AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            Some("/work"),
        )
        .unwrap(),
        mcp_servers: None,
    }
}

fn diagnostics(facts: &[PublicWireFact]) -> Vec<&str> {
    facts
        .iter()
        .filter_map(|fact| match fact {
            PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic {
                classification,
            }) => Some(classification.as_str()),
            _ => None,
        })
        .collect()
}

fn codex_findings() -> Vec<String> {
    let methods = codex_methods();
    let schema = methods.as_object().unwrap();
    let mut findings = Vec::new();
    let allowed: BTreeSet<&str> = CODEX_METHODS_OUTSIDE_SCHEMA
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let mut server_fields = BTreeSet::new();
    for entry in schema.values() {
        if matches!(
            entry["direction"].as_str(),
            Some("serverNotification" | "serverRequest")
        ) {
            server_fields.extend(entry["params"].as_object().unwrap().keys().cloned());
        }
    }
    for (path, source) in sources(
        &workspace().join("crates/gent-drivers/src"),
        &is_codex_source,
    ) {
        let file = path
            .strip_prefix(workspace())
            .unwrap_or(&path)
            .display()
            .to_string();
        for value in literals(&source) {
            if looks_like_method(&value)
                && !schema.contains_key(&value)
                && !allowed.contains(value.as_str())
            {
                findings.push(format!("method-removed {value} is used by {file} but absent from the pinned codex schema"));
            }
            if let Some(field) = value.strip_prefix("/params") {
                let field = if field.is_empty() { "/" } else { field };
                if !server_fields.contains(field) {
                    findings.push(format!("field-removed {value} is read by {file} but no pinned codex server message has it"));
                }
            }
        }
    }
    for (method, entry) in schema {
        match entry["direction"].as_str() {
            Some("serverNotification") => {
                let facts = normalize_public_frame(
                    PublicProvider::Codex,
                    &json!({"method": method, "params": {}}),
                );
                if diagnostics(&facts).contains(&"unsupportedCodexNotification") {
                    findings.push(format!("notification-new {method} → handle it in public_protocol/codex_protocol.rs or add it to housekeeping in codex_protocol_support.rs"));
                }
            }
            Some("serverRequest") => {
                let (mut driver, _) =
                    CodexTurnDriver::start(codex_config(), "probe", None).unwrap();
                let frame = serde_json::to_vec(
                    &json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": {}}),
                )
                .unwrap();
                let effects = driver.receive(&frame).unwrap();
                let generic = effects.iter().any(|effect| matches!(effect, CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification })) if classification == "unsupportedCodexServerRequest"));
                if generic {
                    findings.push(format!("server-request-new {method} → answer it in codex_control.rs or codex_client_request.rs (it is currently rejected with a JSON-RPC error)"));
                }
            }
            _ => {}
        }
    }
    findings
}

fn flags(source: &str) -> BTreeSet<String> {
    literals(source)
        .into_iter()
        .filter(|value| {
            value.starts_with("--")
                && value.len() > 2
                && value[2..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .collect()
}

fn argument_lists(source: &str, marker: &str) -> Vec<Vec<String>> {
    source
        .lines()
        .filter(|line| line.contains(marker) && line.contains("&["))
        .map(|line| literals(&line[line.find("&[").unwrap()..]))
        .collect()
}

fn claude_findings() -> Vec<String> {
    let cli = snapshot("claude", "cli.json");
    let probes = snapshot("claude", "launch-probe.json");
    let mut findings = Vec::new();
    let mut probed = BTreeSet::new();
    for (name, probe) in probes.as_object().unwrap() {
        if probe["handshake"] != "success" || probe["stderr"] != "" {
            findings.push(format!(
                "launch-probe {name} did not complete initialize with clean stderr: {probe}"
            ));
        }
        probed.extend(
            probe["argv"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    let options = cli["<root>"]["options"].as_object().unwrap();
    for file in [
        "crates/gent-drivers/src/launch_spec.rs",
        "crates/gent-drivers/src/claude_turn_options.rs",
    ] {
        for flag in flags(&production_source(&workspace().join(file))) {
            if !options.contains_key(&flag) && !probed.contains(&flag) {
                findings.push(format!("flag-removed {flag} is built by {file} but is neither in pinned claude --help nor in an accepted launch probe"));
            }
        }
    }
    let auth = production_source(&workspace().join("crates/gentd/src/provider_auth_process.rs"));
    for (marker, provider) in [
        ("ProviderAuthProvider::Claude", "claude"),
        ("ProviderAuthProvider::Codex", "codex"),
    ] {
        let surface = snapshot(provider, "cli.json");
        for arguments in argument_lists(&auth, marker) {
            findings.extend(cli_path_findings(provider, &surface, &arguments));
        }
    }
    findings
}

fn cli_path_findings(provider: &str, surface: &Value, arguments: &[String]) -> Vec<String> {
    let mut findings = Vec::new();
    let mut path: Vec<&str> = Vec::new();
    for argument in arguments {
        let command = if path.is_empty() {
            "<root>".to_owned()
        } else {
            path.join(" ")
        };
        let help = &surface[&command];
        if argument.starts_with("--") {
            if help["options"].get(argument).is_none() {
                findings.push(format!(
                    "flag-removed {provider} {command} {argument} is used by gentd provider auth"
                ));
            }
        } else if help["subcommands"].get(argument).is_none() {
            findings.push(format!(
                "subcommand-removed {provider} {command} {argument} is used by gentd provider auth"
            ));
            return findings;
        } else {
            path.push(argument);
        }
    }
    findings
}

fn codex_launch_findings() -> Vec<String> {
    let cli = snapshot("codex", "cli.json");
    let probe = &snapshot("codex", "launch-probe.json")["appServer"];
    let arguments = gent_drivers::launch_spec::codex_app_server_arguments();
    let mut findings = Vec::new();
    if probe["argv"] != json!(arguments) || probe["handshake"] != "success" || probe["stderr"] != ""
    {
        findings.push(format!("launch-probe codex app-server argv {arguments:?} was not accepted with clean stderr by the pinned codex: {probe}"));
    }
    let Some((subcommand, options)) = arguments.split_first() else {
        return vec!["codex launch argv is empty".into()];
    };
    if cli["<root>"]["subcommands"].get(subcommand).is_none() {
        findings.push(format!(
            "subcommand-removed codex {subcommand} is launched by launch_spec.rs"
        ));
    }
    let surface = cli[subcommand.as_str()]["options"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for option in options.iter().filter(|option| option.starts_with('-')) {
        let known = surface.contains_key(option)
            || surface
                .values()
                .any(|detail| detail["short"] == json!(option));
        if !known {
            findings.push(format!(
                "flag-removed codex {subcommand} {option} is launched by launch_spec.rs"
            ));
        }
    }
    findings
}

fn claurst_findings() -> Vec<String> {
    let protocol = snapshot("claurst", "protocol.json");
    let strings = |key: &str| -> BTreeSet<String> {
        protocol[key]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let schema_updates = strings("schemaSessionUpdates");
    let emitted = strings("emittedSessionUpdates");
    let known_methods: BTreeSet<String> = strings("schemaAgentMethods")
        .union(&strings("schemaClientMethods"))
        .cloned()
        .collect();
    let served: BTreeSet<String> = strings("servedRequests")
        .union(&strings("servedNotifications"))
        .cloned()
        .collect();
    let mut findings = Vec::new();
    let updates =
        production_source(&workspace().join("crates/gentd/src/claurst_acp_transport_updates.rs"));
    let dispatch = updates
        .split("fn session_update_fact")
        .nth(1)
        .and_then(|body| body.split("\n    fn ").next())
        .expect("gentd claurst session_update_fact dispatch");
    let handled: BTreeSet<String> = dispatch
        .lines()
        .filter(|line| line.contains("=>") && line.trim_start().starts_with('"'))
        .flat_map(|line| literals(&line[..line.find("=>").unwrap()]))
        .collect();
    for kind in &handled {
        if !schema_updates.contains(kind) {
            findings.push(format!("session-update-removed {kind} is handled by gentd claurst_acp_transport_updates.rs but absent from the pinned ACP schema"));
        }
    }
    for kind in emitted.difference(&handled) {
        findings.push(format!("session-update-new {kind} is emitted by pinned Claurst but not handled by gentd claurst_acp_transport_updates.rs"));
    }
    for file in ["claurst_acp_transport.rs", "claurst_acp_transport_io.rs"] {
        for value in literals(&production_source(
            &workspace().join("crates/gentd/src").join(file),
        )) {
            if (value == "initialize" || value.starts_with("session/"))
                && !known_methods.contains(&value)
            {
                findings.push(format!("method-removed {value} is used by gentd {file} but absent from the pinned ACP schema"));
            } else if value.starts_with("session/")
                && protocol["schemaAgentMethods"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|method| method == &value)
                && !served.contains(&value)
            {
                findings.push(format!("method-unserved {value} is sent by gentd {file} but pinned Claurst does not serve it"));
            }
        }
    }
    findings
}

fn claude_command_findings() -> Vec<String> {
    use gent_drivers::claude_commands::{CLAUDE_BUILTIN_COMMANDS, ClaudeBuiltin, claude_builtin};
    let commands = snapshot("claude", "commands.json");
    let reserved = gent_core::reserved_names(Some(gent_types::AgentChatProvider::Claude));
    let mut findings = Vec::new();
    for (name, detail) in commands.as_object().unwrap() {
        if name.starts_with("__") {
            continue;
        }
        match claude_builtin(name) {
            None => findings.push(format!("command-builtin-new claude /{name} has no disposition → add it to CLAUDE_BUILTIN_COMMANDS (claude_commands.rs)")),
            Some(ClaudeBuiltin::Native) if reserved.contains(&name.as_str()) => findings.push(format!("command-reserved claude /{name} is native but Gent reserves that name → mark it GentReserved or rename the Gent command")),
            Some(_) => {}
        }
        for alias in detail["aliases"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
        {
            if claude_builtin(name) == Some(ClaudeBuiltin::Native) && reserved.contains(&alias) {
                findings.push(format!(
                    "command-reserved claude /{name} alias /{alias} is shadowed by a Gent command"
                ));
            }
        }
    }
    for (name, disposition) in CLAUDE_BUILTIN_COMMANDS {
        if *disposition == ClaudeBuiltin::GentReserved && !reserved.contains(name) {
            findings.push(format!("command-reserved claude /{name} is GentReserved but gent-core no longer reserves it"));
        }
    }
    findings
}

#[test]
fn pinned_provider_contracts_match_what_the_drivers_depend_on() {
    let findings: Vec<String> = [
        codex_findings(),
        codex_launch_findings(),
        claude_findings(),
        claude_command_findings(),
        claurst_findings(),
    ]
    .concat();
    assert!(
        findings.is_empty(),
        "provider contract drift against fixtures/provider-contracts/pins.json:\n{}",
        findings.join("\n")
    );
}

#[test]
fn drift_scanner_reads_literals_methods_and_fields() {
    assert_eq!(
        literals(r#"a("turn/start") b('"') c("x\"y")"#),
        ["turn/start", "xy"]
    );
    assert!(looks_like_method("item/commandExecution/requestApproval"));
    assert!(!looks_like_method("/params/turn"));
    assert!(!looks_like_method("text/plain; charset"));
    let surface = json!({"<root>": {"subcommands": {"auth": []}, "options": {}}, "auth": {"subcommands": {"status": []}, "options": {}}, "auth status": {"subcommands": {}, "options": {"--json": {}}}});
    assert!(
        cli_path_findings(
            "claude",
            &surface,
            &["auth".into(), "status".into(), "--json".into()]
        )
        .is_empty()
    );
    assert_eq!(
        cli_path_findings("claude", &surface, &["auth".into(), "logout".into()]),
        ["subcommand-removed claude auth logout is used by gentd provider auth"]
    );
}
