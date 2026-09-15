use clap::CommandFactory;

use super::Args;

fn undocumented(command: &clap::Command, path: &str, missing: &mut Vec<String>) {
    for argument in command.get_arguments() {
        let id = argument.get_id().as_str();
        if matches!(id, "help" | "version") || argument.is_hide_set() {
            continue;
        }
        if argument.get_help().is_none() && argument.get_long_help().is_none() {
            missing.push(format!("{path} <{id}>"));
        }
    }
    for subcommand in command.get_subcommands() {
        let name = subcommand.get_name();
        if name == "help" {
            continue;
        }
        let child = format!("{path} {name}");
        if subcommand.get_about().is_none() && subcommand.get_long_about().is_none() {
            missing.push(child.clone());
        }
        undocumented(subcommand, &child, missing);
    }
}

#[test]
fn every_command_and_argument_has_help_text() {
    let mut command = Args::command();
    command.build();
    let mut missing = Vec::new();
    undocumented(&command, "gent", &mut missing);
    assert!(missing.is_empty(), "missing help:\n{}", missing.join("\n"));
}

#[test]
fn conversation_browser_help_does_not_promise_read_only_access() {
    let command = Args::command();
    let help = command
        .get_arguments()
        .find(|argument| argument.get_id() == "conversations")
        .and_then(clap::Arg::get_help)
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(!help.contains("read-only"), "{help}");
    assert!(help.contains("conversation"), "{help}");
}

#[test]
fn effort_help_defers_to_the_model_catalog_instead_of_a_fixed_list() {
    for path in [
        &["chat", "send"][..],
        &["chat", "switch"][..],
        &["plan", "start"][..],
    ] {
        let mut command = Args::command();
        let mut current = &mut command;
        for name in path {
            current = current.find_subcommand_mut(name).unwrap();
        }
        let help = current.render_long_help().to_string();
        assert!(!help.contains("ultra"), "{path:?}\n{help}");
        assert!(!help.contains("xhigh"), "{path:?}\n{help}");
    }
    let root = Args::command().render_long_help().to_string();
    assert!(!root.contains("ultra"), "{root}");
}
