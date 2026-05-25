//! kittwm keybinding configuration.
//!
//! v1 intentionally uses a tiny line-oriented language so operators can edit
//! keybindings without pulling in a full scripting runtime:
//!
//! ```text
//! # comments and blank lines are ignored
//! prefix C-a
//! bind c workspace.new
//! bind | split.vertical.launcher
//! bind l swap.right
//! bind C-h focus.left
//! ```
//!
//! `prefix` makes every following non-prefixed `bind` a two-key chord. A bind
//! can also spell an explicit two-key chord with a space: `bind C-a c ...`.

use anyhow::{anyhow, Result};
use std::fmt;
use std::path::Path;

/// Modifier flags for a key spec.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyMods {
    /// Control modifier.
    pub ctrl: bool,
    /// Alt / Meta modifier.
    pub alt: bool,
    /// Shift modifier.
    pub shift: bool,
}

/// A single key press in a chord.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeySpec {
    /// Modifiers held for this key.
    pub mods: KeyMods,
    /// The key name, lower-case for symbolic keys, literal for punctuation.
    pub key: String,
}

impl KeySpec {
    /// Parse one key token: `a`, `|`, `C-a`, `M-x`, `S-Enter`, `F12`.
    pub fn parse(token: &str) -> Result<Self> {
        let mut mods = KeyMods::default();
        if token.is_empty() {
            return Err(anyhow!("empty key token"));
        }
        // A bare dash is a valid punctuation key, not a modifier separator.
        if token == "-" {
            return Ok(Self {
                mods,
                key: "-".to_string(),
            });
        }
        let mut parts: Vec<&str> = token.split('-').collect();
        let key = parts.pop().unwrap_or("");
        if key.is_empty() {
            return Err(anyhow!("missing key in {token:?}"));
        }
        for m in parts {
            match m.to_ascii_lowercase().as_str() {
                "c" | "ctrl" | "control" => mods.ctrl = true,
                "m" | "meta" | "alt" => mods.alt = true,
                "s" | "shift" => mods.shift = true,
                other => return Err(anyhow!("unknown modifier {other:?} in {token:?}")),
            }
        }
        Ok(Self {
            mods,
            key: normalize_key_name(key),
        })
    }
}

fn normalize_key_name(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => "enter".into(),
        "esc" | "escape" => "escape".into(),
        "space" => "space".into(),
        "tab" => "tab".into(),
        other => other.to_string(),
    }
}

impl fmt::Display for KeySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            write!(f, "C-")?;
        }
        if self.mods.alt {
            write!(f, "M-")?;
        }
        if self.mods.shift {
            write!(f, "S-")?;
        }
        write!(f, "{}", self.key)
    }
}

/// WM action vocabulary exposed by the config language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// Create a new virtual workspace.
    WorkspaceNew,
    /// Switch to next virtual workspace.
    WorkspaceNext,
    /// Switch to previous virtual workspace.
    WorkspacePrev,
    /// Split current tiled view vertically and open the launcher in the new pane.
    SplitVerticalLauncher,
    /// Split current tiled view horizontally and open the launcher in the new pane.
    SplitHorizontalLauncher,
    /// Open launcher.
    Launch,
    /// Open backend/window picker.
    PickerOpen,
    /// Toggle fullscreen state for focused tile/window.
    FullscreenToggle,
    /// Toggle floating state for focused tile/window.
    FloatToggle,
    /// Toggle split orientation for future tiling.
    ToggleSplit,
    /// Balance window layout.
    BalanceWindows,
    /// Reload config/keymap state.
    ReloadConfig,
    /// Swap focused tile right.
    SwapRight,
    /// Swap focused tile left.
    SwapLeft,
    /// Swap focused tile up.
    SwapUp,
    /// Swap focused tile down.
    SwapDown,
    /// Focus right.
    FocusRight,
    /// Focus left.
    FocusLeft,
    /// Focus up.
    FocusUp,
    /// Focus down.
    FocusDown,
    /// Quit kittwm.
    Quit,
    /// Any future/custom action token.
    Custom(String),
}

impl Action {
    /// Parse an action token.
    pub fn parse(token: &str) -> Self {
        match token {
            "workspace.new" | "workspace-create" => Self::WorkspaceNew,
            "workspace.next" => Self::WorkspaceNext,
            "workspace.prev" => Self::WorkspacePrev,
            "split.vertical.launcher" | "split-vertical-launcher" => Self::SplitVerticalLauncher,
            "split.horizontal.launcher" | "split-horizontal-launcher" => {
                Self::SplitHorizontalLauncher
            }
            "launch" | "launcher.open" => Self::Launch,
            "picker.open" | "window.picker" => Self::PickerOpen,
            "fullscreen" | "fullscreen.toggle" => Self::FullscreenToggle,
            "float.toggle" | "floating.toggle" => Self::FloatToggle,
            "toggle.split" | "split.toggle" => Self::ToggleSplit,
            "balance.windows" | "balance" => Self::BalanceWindows,
            "reload.config" | "reload" => Self::ReloadConfig,
            "swap.right" => Self::SwapRight,
            "swap.left" => Self::SwapLeft,
            "swap.up" => Self::SwapUp,
            "swap.down" => Self::SwapDown,
            "focus.right" => Self::FocusRight,
            "focus.left" => Self::FocusLeft,
            "focus.up" => Self::FocusUp,
            "focus.down" => Self::FocusDown,
            "quit" => Self::Quit,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::WorkspaceNew => "workspace.new",
            Self::WorkspaceNext => "workspace.next",
            Self::WorkspacePrev => "workspace.prev",
            Self::SplitVerticalLauncher => "split.vertical.launcher",
            Self::SplitHorizontalLauncher => "split.horizontal.launcher",
            Self::Launch => "launch",
            Self::PickerOpen => "picker.open",
            Self::FullscreenToggle => "fullscreen.toggle",
            Self::FloatToggle => "float.toggle",
            Self::ToggleSplit => "toggle.split",
            Self::BalanceWindows => "balance.windows",
            Self::ReloadConfig => "reload.config",
            Self::SwapRight => "swap.right",
            Self::SwapLeft => "swap.left",
            Self::SwapUp => "swap.up",
            Self::SwapDown => "swap.down",
            Self::FocusRight => "focus.right",
            Self::FocusLeft => "focus.left",
            Self::FocusUp => "focus.up",
            Self::FocusDown => "focus.down",
            Self::Quit => "quit",
            Self::Custom(s) => s,
        };
        write!(f, "{s}")
    }
}

/// One key binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    /// Chord sequence, usually `[prefix, key]`.
    pub chord: Vec<KeySpec>,
    /// Action triggered by the chord.
    pub action: Action,
}

impl Binding {
    /// Render chord as user-facing key tokens.
    pub fn chord_string(&self) -> String {
        self.chord
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Complete keymap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Keymap {
    /// Prefix key, if any.
    pub prefix: Option<KeySpec>,
    /// All configured bindings.
    pub bindings: Vec<Binding>,
}

impl Keymap {
    /// Parse keymap from text.
    pub fn parse(src: &str) -> Result<Self> {
        let mut prefix: Option<KeySpec> = None;
        let mut bindings = Vec::new();
        for (idx, raw) in src.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts.as_slice() {
                ["prefix", key] => prefix = Some(KeySpec::parse(key)?),
                ["bind", key, action] => {
                    let mut chord = Vec::new();
                    if let Some(p) = &prefix {
                        chord.push(p.clone());
                    }
                    chord.push(KeySpec::parse(key)?);
                    bindings.push(Binding { chord, action: Action::parse(action) });
                }
                ["bind", first, second, action] => {
                    bindings.push(Binding {
                        chord: vec![KeySpec::parse(first)?, KeySpec::parse(second)?],
                        action: Action::parse(action),
                    });
                }
                _ => return Err(anyhow!("line {line_no}: expected `prefix KEY`, `bind KEY ACTION`, or `bind KEY KEY ACTION`, got {line:?}")),
            }
        }
        Ok(Self { prefix, bindings })
    }

    /// Load from a file path.
    pub fn load(path: &Path) -> Result<Self> {
        let src = std::fs::read_to_string(path)?;
        Self::parse(&src)
    }

    /// Look up an action for an exact chord.
    pub fn action_for_chord(&self, chord: &[KeySpec]) -> Option<&Action> {
        self.bindings
            .iter()
            .find(|b| b.chord.as_slice() == chord)
            .map(|b| &b.action)
    }

    /// Render as a stable table.
    pub fn render_table(&self) -> String {
        let mut out = String::new();
        out.push_str("kittwm keymap\n");
        out.push_str("============\n");
        match &self.prefix {
            Some(p) => out.push_str(&format!("prefix: {p}\n")),
            None => out.push_str("prefix: <none>\n"),
        }
        out.push_str("\nbindings:\n");
        for b in &self.bindings {
            out.push_str(&format!("  {:<12} -> {}\n", b.chord_string(), b.action));
        }
        out
    }
}

/// Tmux-like default kittwm keymap.
pub fn default_keymap() -> Keymap {
    Keymap::parse(DEFAULT_KEYMAP).expect("built-in keymap must parse")
}

/// Built-in keymap source, printed in docs/tests.
pub const DEFAULT_KEYMAP: &str = r#"
# kittwm default keymap: tmux-like Ctrl-A prefix
prefix C-a
bind c workspace.new
bind n workspace.next
bind p workspace.prev
bind | split.vertical.launcher
bind - split.horizontal.launcher
bind Enter launch
bind d launch
bind g launch
bind Space picker.open
bind f fullscreen.toggle
bind t float.toggle
bind e toggle.split
bind = balance.windows
bind r reload.config
bind l swap.right
bind h swap.left
bind k swap.up
bind j swap.down
bind C-l focus.right
bind C-h focus.left
bind C-k focus.up
bind C-j focus.down
bind q quit
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keymap_has_requested_chords() {
        let km = default_keymap();
        assert_eq!(km.prefix.as_ref().unwrap().to_string(), "C-a");
        let rendered = km.render_table();
        assert!(rendered.contains("C-a c"));
        assert!(rendered.contains("workspace.new"));
        assert!(rendered.contains("C-a |"));
        assert!(rendered.contains("split.vertical.launcher"));
        assert!(rendered.contains("C-a d"));
        assert!(rendered.contains("C-a g"));
        assert!(rendered.contains("launch"));
        assert!(rendered.contains("C-a f"));
        assert!(rendered.contains("fullscreen.toggle"));
        assert!(rendered.contains("C-a t"));
        assert!(rendered.contains("float.toggle"));
        assert!(rendered.contains("C-a e"));
        assert!(rendered.contains("toggle.split"));
        assert!(rendered.contains("C-a ="));
        assert!(rendered.contains("balance.windows"));
        assert!(rendered.contains("C-a r"));
        assert!(rendered.contains("reload.config"));
        assert!(rendered.contains("C-a C-h"));
        assert!(rendered.contains("focus.left"));
    }

    #[test]
    fn parses_explicit_chord_and_custom_action() {
        let km = Keymap::parse("bind C-a x custom.do-thing\n").unwrap();
        assert_eq!(km.bindings[0].chord_string(), "C-a x");
        assert_eq!(km.bindings[0].action.to_string(), "custom.do-thing");
    }
}
