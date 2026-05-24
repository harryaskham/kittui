//! Shared native kittwm shortcut/help list.

/// Native kittwm shortcut rows, shared by `C-a ?` and CLI inspection.
pub const NATIVE_SHORTCUTS: &[&str] = &[
    "kittwm shortcuts",
    "C-a Enter / C-a t  launch terminal",
    "C-a ?              toggle this help",
    "C-a % / C-a |      split columns",
    "C-a -              split rows",
    "C-a Tab            focus next pane",
    "C-a x              close pane (last pane returns to empty workspace)",
    "C-a +/-            resize focused pane",
    "C-a [ / C-a ]      move focused pane",
    "C-a b              balance panes",
    "Ctrl-]             exit kittwm",
];

/// Render the shortcut list as newline-delimited text.
pub fn render_native_shortcuts() -> String {
    let mut out = NATIVE_SHORTCUTS.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_include_first_launch_actions() {
        let text = render_native_shortcuts();
        assert!(text.contains("launch terminal"), "{text}");
        assert!(text.contains("toggle this help"), "{text}");
        assert!(text.contains("Ctrl-]"), "{text}");
    }
}
