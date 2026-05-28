//! Shared native kittwm shortcut/help list.

/// One native kittwm shortcut entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeShortcut {
    /// Stable machine-readable action id.
    pub id: &'static str,
    /// Human-readable key chord(s).
    pub keys: &'static str,
    /// Human-readable shortcut description.
    pub description: &'static str,
}

/// Native kittwm shortcut entries, shared by text and JSON inspection.
pub const NATIVE_SHORTCUT_ENTRIES: &[NativeShortcut] = &[
    NativeShortcut {
        id: "launch_terminal",
        keys: "C-a Enter",
        description: "launch terminal",
    },
    NativeShortcut {
        id: "open_launcher",
        keys: "C-a g",
        description: "open launcher",
    },
    NativeShortcut {
        id: "toggle_help",
        keys: "C-a ?",
        description: "toggle this help",
    },
    NativeShortcut {
        id: "switch_workspace",
        keys: "C-a 1..9",
        description: "switch/create workspace on demand",
    },
    NativeShortcut {
        id: "split_columns",
        keys: "C-a % / C-a |",
        description: "split columns",
    },
    NativeShortcut {
        id: "split_rows",
        keys: "C-a -",
        description: "split rows",
    },
    NativeShortcut {
        id: "toggle_split",
        keys: "C-a e",
        description: "toggle current split vertical/horizontal",
    },
    NativeShortcut {
        id: "toggle_floating",
        keys: "C-a t",
        description: "toggle floating mode",
    },
    NativeShortcut {
        id: "toggle_fullscreen",
        keys: "C-a f",
        description: "toggle fullscreen",
    },
    NativeShortcut {
        id: "focus_next",
        keys: "C-a Tab",
        description: "focus next pane",
    },
    NativeShortcut {
        id: "close_pane",
        keys: "C-a x",
        description: "close pane (last pane returns to empty workspace)",
    },
    NativeShortcut {
        id: "resize_focused_pane",
        keys: "C-a +/-",
        description: "resize focused pane",
    },
    NativeShortcut {
        id: "move_focused_pane",
        keys: "C-a [ / C-a ]",
        description: "move focused pane",
    },
    NativeShortcut {
        id: "balance_panes",
        keys: "C-a b",
        description: "balance panes",
    },
    NativeShortcut {
        id: "exit_kittwm",
        keys: "Ctrl-C×3 then y / Ctrl-]",
        description: "confirm and exit kittwm",
    },
];

/// External daily-driver command hints appended to the shortcut overlay.
pub const NATIVE_SHORTCUT_COMMAND_HINTS: &[&str] = &[
    "outside: kittwm info · kittwm quickstart · kittwm examples · kittwm cheat",
    "outside: kittwm panes · events 1000 · help panes",
];

/// Native kittwm shortcut rows, shared by `C-a ?` and CLI inspection.
pub const NATIVE_SHORTCUTS: &[&str] = &[
    "kittwm shortcuts",
    "C-a Enter          launch terminal",
    "C-a g              open launcher",
    "C-a ?              toggle this help",
    "C-a 1..9           switch/create workspace on demand",
    "C-a % / C-a |      split columns",
    "C-a -              split rows",
    "C-a e              toggle current split vertical/horizontal",
    "C-a t              toggle floating mode",
    "C-a f              toggle fullscreen",
    "C-a Tab            focus next pane",
    "C-a x              close pane (last pane returns to empty workspace)",
    "C-a +/-            resize focused pane",
    "C-a [ / C-a ]      move focused pane",
    "C-a b              balance panes",
    "Ctrl-C×3 then y    confirm and exit kittwm",
    "Ctrl-]             emergency/direct exit",
    NATIVE_SHORTCUT_COMMAND_HINTS[0],
    NATIVE_SHORTCUT_COMMAND_HINTS[1],
];

/// Render the shortcut list as newline-delimited text.
pub fn render_native_shortcuts() -> String {
    let mut out = NATIVE_SHORTCUTS.join("\n");
    out.push('\n');
    out
}

/// Render the shortcut list as machine-readable JSON.
pub fn render_native_shortcuts_json() -> String {
    let shortcuts: Vec<_> = NATIVE_SHORTCUT_ENTRIES
        .iter()
        .map(|entry| {
            serde_json::json!({
                "id": entry.id,
                "keys": entry.keys,
                "description": entry.description,
            })
        })
        .collect();
    format!(
        "{}\n",
        serde_json::json!({
            "schema_version": 1,
            "kind": "kittwm-native-shortcuts",
            "shortcuts": shortcuts,
        })
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_include_first_launch_actions() {
        let text = render_native_shortcuts();
        assert!(text.contains("launch terminal"), "{text}");
        assert!(text.contains("open launcher"), "{text}");
        assert!(text.contains("toggle this help"), "{text}");
        assert!(text.contains("C-a 1..9"), "{text}");
        assert!(text.contains("switch/create workspace"), "{text}");
        assert!(text.contains("Ctrl-C×3 then y"), "{text}");
        assert!(text.contains("Ctrl-]"), "{text}");
        assert!(text.contains("kittwm info"), "{text}");
        assert!(text.contains("kittwm cheat"), "{text}");
    }

    #[test]
    fn shortcuts_json_includes_first_launch_actions() {
        let value: serde_json::Value =
            serde_json::from_str(&render_native_shortcuts_json()).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["kind"], "kittwm-native-shortcuts");
        let shortcuts = value["shortcuts"].as_array().unwrap();
        assert!(shortcuts
            .iter()
            .any(|entry| entry["id"] == "launch_terminal"));
        assert!(shortcuts.iter().any(|entry| entry["id"] == "open_launcher"));
        assert!(shortcuts.iter().any(|entry| entry["id"] == "toggle_help"));
        assert!(shortcuts
            .iter()
            .any(|entry| entry["id"] == "switch_workspace" && entry["keys"] == "C-a 1..9"));
        assert!(shortcuts
            .iter()
            .any(|entry| entry["keys"] == "Ctrl-C×3 then y / Ctrl-]"));
        assert!(shortcuts
            .iter()
            .all(|entry| entry["id"] != "daily_driver_commands"));
    }
}
