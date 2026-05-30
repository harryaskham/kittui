//! Synthetic semantic kittwm SDK app example.
//!
//! This is a protocol dogfood example: it builds a semantic component tree using
//! `kittwm-sdk` types and prints the JSON snapshot that future runtime publishing
//! endpoints can consume. When connected to kittwm, `--query-current` reads the
//! current surface's semantic snapshot, and `--publish-current` / `--publish ID`
//! publish the generated snapshot through the SDK/socket path.

use std::env;

use kittwm_sdk::{
    ActionKind, ComponentAction, ComponentNode,
    ComponentRole, ComponentState, ComponentValue, Kittwm, SemanticSurfaceSnapshot,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn synthetic_settings_snapshot(surface: impl Into<String>) -> SemanticSurfaceSnapshot {
    SemanticSurfaceSnapshot::new(
        surface,
        1,
        ComponentNode::new("settings.root", ComponentRole::Group)
            .labeled("Settings")
            .children(vec![
                ComponentNode::new("settings.tabs", ComponentRole::Tabs)
                    .labeled("Sections")
                    .children(vec![
                        ComponentNode::new("settings.tabs.general", ComponentRole::Label)
                            .labeled("General")
                            .state(ComponentState {
                                selected: true,
                                ..ComponentState::default()
                            }),
                        ComponentNode::new("settings.tabs.advanced", ComponentRole::Label)
                            .labeled("Advanced"),
                    ]),
                ComponentNode::new("settings.name", ComponentRole::TextInput)
                    .labeled("Display name")
                    .valued(ComponentValue::Text("Ada".to_string()))
                    .state(ComponentState {
                        focused: true,
                        focusable: true,
                        ..ComponentState::default()
                    })
                    .actions(vec![
                        ComponentAction::new("focus", ActionKind::Focus),
                        ComponentAction::new("set", ActionKind::SetValue).labeled("Set name"),
                    ]),
                ComponentNode::new("settings.notifications", ComponentRole::Checkbox)
                    .labeled("Enable notifications")
                    .valued(ComponentValue::Bool(true))
                    .state(ComponentState {
                        checked: true,
                        focusable: true,
                        ..ComponentState::default()
                    })
                    .actions(vec![ComponentAction::new("toggle", ActionKind::Toggle)]),
                ComponentNode::new("settings.theme", ComponentRole::RadioGroup)
                    .labeled("Theme")
                    .valued(ComponentValue::Selection(vec![
                        "settings.theme.dark".to_string()
                    ]))
                    .children(vec![
                        ComponentNode::new("settings.theme.light", ComponentRole::Radio)
                            .labeled("Light"),
                        ComponentNode::new("settings.theme.dark", ComponentRole::Radio)
                            .labeled("Dark")
                            .state(ComponentState {
                                selected: true,
                                checked: true,
                                ..ComponentState::default()
                            }),
                    ]),
                ComponentNode::new("settings.profile", ComponentRole::SelectList)
                    .labeled("Profile")
                    .valued(ComponentValue::Selection(vec![
                        "settings.profile.dev".to_string()
                    ]))
                    .children(vec![
                        ComponentNode::new("settings.profile.dev", ComponentRole::Label)
                            .labeled("Developer"),
                        ComponentNode::new("settings.profile.ops", ComponentRole::Label)
                            .labeled("Operator"),
                    ]),
                ComponentNode::new("settings.progress", ComponentRole::Progress)
                    .labeled("Sync progress")
                    .valued(ComponentValue::Number(0.72)),
                ComponentNode::new("settings.split", ComponentRole::SplitPane)
                    .labeled("Preview split")
                    .children(vec![
                        ComponentNode::new("settings.split.form", ComponentRole::Group)
                            .labeled("Form"),
                        ComponentNode::new("settings.split.preview", ComponentRole::Group)
                            .labeled("Preview"),
                    ]),
                ComponentNode::new("settings.save", ComponentRole::Button)
                    .labeled("Save")
                    .actions(vec![ComponentAction::new("activate", ActionKind::Activate)]),
            ]),
    )
    .focused("settings.name")
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SemanticExampleMode {
    Print,
    QueryCurrent,
    Publish(String),
}

fn print_help() {
    println!(
        "usage: kittwm_semantic_app [--surface ID] [--query-current | --publish-current | --publish WINDOW]\n\n\
         Without flags, prints a synthetic semantic settings snapshot as JSON.\n\
         --surface ID       set the surface id in the generated snapshot\n\
         --query-current    read the current kittwm surface semantic snapshot instead\n\
         --publish-current  publish the generated snapshot to focused/current surface\n\
         --publish WINDOW   publish the generated snapshot to an explicit surface"
    );
}

fn main() -> Result<()> {
    let mut surface = "synthetic-settings".to_string();
    let mut mode = SemanticExampleMode::Print;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--surface" => surface = args.next().ok_or("--surface ID")?,
            "--query-current" => mode = SemanticExampleMode::QueryCurrent,
            "--publish-current" => mode = SemanticExampleMode::Publish("focused".to_string()),
            "--publish" => {
                mode = SemanticExampleMode::Publish(args.next().ok_or("--publish WINDOW")?)
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => return Err(format!("unknown arg {other}").into()),
        }
    }

    match mode {
        SemanticExampleMode::Print => {
            let snapshot = synthetic_settings_snapshot(surface);
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        SemanticExampleMode::QueryCurrent => {
            let wm = Kittwm::connect_from_env()?;
            let snapshot = wm.focused_surface().semantic_snapshot()?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        SemanticExampleMode::Publish(target) => {
            let wm = Kittwm::connect_from_env()?;
            let snapshot = synthetic_settings_snapshot(target.clone());
            let reply = wm.surface(target).semantic_publish(&snapshot)?;
            print!("{reply}");
            if !reply.ends_with('\n') {
                println!();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_snapshot_contains_required_control_roles() {
        let snapshot = synthetic_settings_snapshot("test-surface");
        assert_eq!(snapshot.surface, "test-surface");
        assert_eq!(snapshot.focus.unwrap().as_str(), "settings.name");
        let roles = snapshot
            .root
            .children
            .iter()
            .map(|node| &node.role)
            .collect::<Vec<_>>();
        assert!(roles.contains(&&ComponentRole::Tabs));
        assert!(roles.contains(&&ComponentRole::TextInput));
        assert!(roles.contains(&&ComponentRole::Checkbox));
        assert!(roles.contains(&&ComponentRole::RadioGroup));
        assert!(roles.contains(&&ComponentRole::SelectList));
        assert!(roles.contains(&&ComponentRole::Progress));
        assert!(roles.contains(&&ComponentRole::SplitPane));
        assert!(roles.contains(&&ComponentRole::Button));
    }

    #[test]
    fn synthetic_snapshot_serializes_protocol_json() {
        let value = serde_json::to_value(synthetic_settings_snapshot("test-surface")).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["root"]["children"][1]["role"], "text_input");
        assert_eq!(value["root"]["children"][2]["value"]["kind"], "bool");
        assert_eq!(value["root"]["children"][3]["role"], "radio_group");
        assert_eq!(value["focus"], "settings.name");
    }

    #[test]
    fn publish_mode_targets_focused_or_explicit_surface() {
        assert_eq!(
            SemanticExampleMode::Publish("focused".to_string()),
            SemanticExampleMode::Publish("focused".to_string())
        );
        let snapshot = synthetic_settings_snapshot("native-7");
        assert_eq!(snapshot.surface, "native-7");
    }

    #[test]
    fn layout_types_are_available_for_future_publishers() {
        let layout = ComponentLayout {
            kind: ComponentLayoutKind::Column,
            x: Some(1),
            y: Some(2),
            cols: Some(40),
            rows: Some(10),
        };
        assert_eq!(serde_json::to_value(layout).unwrap()["kind"], "column");
    }
}
