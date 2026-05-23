//! Platform accessibility-tree semantic adapter proof.
//!
//! This module intentionally starts with a safe, testable adapter core rather
//! than binding directly to platform FFI. macOS AX / Linux AT-SPI integrations
//! can feed [`AccessibilityNode`] values into this mapper after they associate a
//! captured window with a platform accessibility tree. The first target is macOS
//! AX (`bd-a17062`): the proof below models AX roles/actions, permission
//! diagnostics, sensitive value redaction, and bounded snapshot extraction.

use kittwm_sdk::{
    ActionKind, ComponentAction, ComponentNode, ComponentRole, ComponentState, ComponentValue,
    SemanticSurfaceSnapshot,
};

/// Platform accessibility backend represented by an extracted tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityPlatform {
    /// macOS Accessibility (AXUIElement / AXObserver) tree.
    MacAx,
    /// Linux AT-SPI tree.
    LinuxAtSpi,
}

/// Permission/runtime state for a platform accessibility backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibilityDiagnostics {
    /// Source platform.
    pub platform: AccessibilityPlatform,
    /// Whether the adapter believes accessibility access is currently usable.
    pub available: bool,
    /// Human-readable reason when unavailable or degraded.
    pub reason: Option<String>,
}

impl AccessibilityDiagnostics {
    /// Conservative macOS diagnostic without unsafe AX FFI. The live adapter can
    /// replace this with an AXIsProcessTrusted probe once a safe platform crate
    /// is introduced.
    pub fn mac_ax_unknown() -> Self {
        Self {
            platform: AccessibilityPlatform::MacAx,
            available: false,
            reason: Some(
                "macOS AX permission must be granted to the kittwm host process".to_string(),
            ),
        }
    }

    /// Conservative Linux AT-SPI diagnostic without binding to the desktop bus.
    /// A live adapter can replace this once an AT-SPI client crate is wired.
    pub fn linux_atspi_unavailable(reason: impl Into<String>) -> Self {
        Self {
            platform: AccessibilityPlatform::LinuxAtSpi,
            available: false,
            reason: Some(reason.into()),
        }
    }
}

/// App/window association metadata for a captured accessibility tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibilityWindowAssociation {
    /// Source platform.
    pub platform: AccessibilityPlatform,
    /// Kittwm surface/window id that will receive the semantic snapshot.
    pub surface: String,
    /// Owning process id when known.
    pub pid: Option<u32>,
    /// Platform window id such as CGWindowID / X11 window id when known.
    pub platform_window_id: Option<u64>,
    /// Human-readable window title.
    pub title: String,
}

/// Small platform-neutral accessibility node shape used by the proof mapper.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityNode {
    /// Stable platform-local id or generated path id.
    pub id: String,
    /// Platform role string (`AXButton`, `push button`, `text`, ...).
    pub role: String,
    /// Accessible name/title.
    pub name: Option<String>,
    /// Accessible description/help text.
    pub description: Option<String>,
    /// Current value as text/number/bool-ish string.
    pub value: Option<String>,
    /// Whether the node can receive focus.
    pub focusable: bool,
    /// Whether the node is focused.
    pub focused: bool,
    /// Whether the node is disabled.
    pub disabled: bool,
    /// Whether the node is selected.
    pub selected: bool,
    /// Whether the node is checked/pressed.
    pub checked: bool,
    /// Whether the value is sensitive and must be redacted.
    pub sensitive: bool,
    /// Platform action names advertised for the node.
    pub actions: Vec<String>,
    /// Child accessibility nodes.
    pub children: Vec<AccessibilityNode>,
}

impl AccessibilityNode {
    /// Build a node with a platform role.
    pub fn new(id: impl Into<String>, role: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role: role.into(),
            name: None,
            description: None,
            value: None,
            focusable: false,
            focused: false,
            disabled: false,
            selected: false,
            checked: false,
            sensitive: false,
            actions: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Attach accessible name.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Attach value text.
    pub fn valued(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Mark as focusable.
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    /// Mark as sensitive/redacted.
    pub fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }

    /// Attach children.
    pub fn children(mut self, children: Vec<AccessibilityNode>) -> Self {
        self.children = children;
        self
    }

    /// Attach platform actions.
    pub fn actions(mut self, actions: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.actions = actions.into_iter().map(Into::into).collect();
        self
    }
}

/// Convert a bounded accessibility tree into a kittwm semantic snapshot.
pub fn accessibility_snapshot_from_tree(
    association: &AccessibilityWindowAssociation,
    root: &AccessibilityNode,
) -> SemanticSurfaceSnapshot {
    let root_component =
        accessibility_component_from_node(root, 0, 12, 2_000).unwrap_or_else(|| {
            ComponentNode::new(
                format!("{}.root", association.surface),
                ComponentRole::Group,
            )
        });
    SemanticSurfaceSnapshot::new(association.surface.clone(), 1, root_component)
}

fn accessibility_component_from_node(
    node: &AccessibilityNode,
    depth: usize,
    max_depth: usize,
    remaining_nodes: usize,
) -> Option<ComponentNode> {
    if depth > max_depth || remaining_nodes == 0 || node.id.trim().is_empty() {
        return None;
    }
    let role = accessibility_component_role(&node.role);
    let mut state = ComponentState {
        focusable: node.focusable,
        focused: node.focused,
        disabled: node.disabled,
        selected: node.selected,
        checked: node.checked,
        sensitive: node.sensitive,
        ..ComponentState::default()
    };
    if matches!(role, ComponentRole::Checkbox | ComponentRole::Radio) {
        state.checked = node.checked || node.selected;
    }
    let mut component = ComponentNode::new(node.id.clone(), role)
        .state(state)
        .actions(accessibility_actions(&node.role, &node.actions));
    if let Some(name) = node.name.as_ref().filter(|s| !s.trim().is_empty()) {
        component = component.labeled(name.clone());
    }
    if let Some(description) = node.description.as_ref().filter(|s| !s.trim().is_empty()) {
        component.description = Some(description.clone());
    }
    if let Some(value) = accessibility_value(node, &component.role) {
        component = component.valued(value);
    }
    let mut children = Vec::new();
    let mut remaining = remaining_nodes.saturating_sub(1);
    for child in &node.children {
        if remaining == 0 {
            break;
        }
        if let Some(mapped) =
            accessibility_component_from_node(child, depth + 1, max_depth, remaining)
        {
            remaining = remaining.saturating_sub(1);
            children.push(mapped);
        }
    }
    if !children.is_empty() {
        component = component.children(children);
    }
    Some(component)
}

fn accessibility_component_role(role: &str) -> ComponentRole {
    let role_l = role.to_ascii_lowercase();
    if role_l.contains("button") {
        ComponentRole::Button
    } else if role_l.contains("check") || role_l.contains("toggle") {
        ComponentRole::Checkbox
    } else if role_l.contains("radio group") {
        ComponentRole::RadioGroup
    } else if role_l.contains("radio") {
        ComponentRole::Radio
    } else if role_l.contains("text area") || role_l.contains("textarea") {
        ComponentRole::TextArea
    } else if role_l.contains("text") || role_l.contains("edit") || role_l.contains("field") {
        ComponentRole::TextInput
    } else if role_l.contains("combo") || role_l.contains("list") || role_l.contains("select") {
        ComponentRole::SelectList
    } else if role_l.contains("slider") || role_l.contains("incrementor") {
        ComponentRole::Slider
    } else if role_l.contains("progress") {
        ComponentRole::Progress
    } else if role_l.contains("menu") {
        ComponentRole::Menu
    } else if role_l.contains("table") || role_l.contains("grid") {
        ComponentRole::Table
    } else if role_l.contains("static") || role_l.contains("label") || role_l.contains("heading") {
        ComponentRole::Label
    } else if role_l.contains("window")
        || role_l.contains("frame")
        || role_l.contains("group")
        || role_l.contains("panel")
    {
        ComponentRole::Group
    } else {
        ComponentRole::Custom(format!("accessibility.{role}"))
    }
}

fn accessibility_value(node: &AccessibilityNode, role: &ComponentRole) -> Option<ComponentValue> {
    if node.sensitive {
        return None;
    }
    match role {
        ComponentRole::Checkbox | ComponentRole::Radio => {
            Some(ComponentValue::Bool(node.checked || node.selected))
        }
        ComponentRole::Slider | ComponentRole::Progress => node
            .value
            .as_deref()
            .and_then(|value| value.parse::<f32>().ok())
            .map(ComponentValue::Number),
        ComponentRole::TextInput | ComponentRole::TextArea | ComponentRole::Label => node
            .value
            .as_ref()
            .or(node.name.as_ref())
            .map(|value| ComponentValue::Text(value.clone())),
        _ => node
            .value
            .as_ref()
            .map(|value| ComponentValue::Text(value.clone())),
    }
}

fn accessibility_actions(role: &str, platform_actions: &[String]) -> Vec<ComponentAction> {
    let role_l = role.to_ascii_lowercase();
    let mut out = Vec::new();
    if !platform_actions.is_empty() || role_l.contains("button") || role_l.contains("menu") {
        out.push(ComponentAction::new("activate", ActionKind::Activate));
    }
    if role_l.contains("check") || role_l.contains("toggle") {
        out.push(ComponentAction::new("toggle", ActionKind::Toggle));
    }
    if role_l.contains("text") || role_l.contains("edit") || role_l.contains("field") {
        out.push(ComponentAction::new("set_value", ActionKind::SetValue));
        out.push(ComponentAction::new("insert_text", ActionKind::InsertText));
    }
    if role_l.contains("radio") || role_l.contains("list") || role_l.contains("select") {
        out.push(ComponentAction::new("select", ActionKind::Select));
    }
    if role_l.contains("slider") || role_l.contains("incrementor") {
        out.push(ComponentAction::new("set_value", ActionKind::SetValue));
    }
    if role_l.contains("disclosure") || role_l.contains("tree") {
        out.push(ComponentAction::new("expand", ActionKind::Expand));
        out.push(ComponentAction::new("collapse", ActionKind::Collapse));
    }
    if platform_actions
        .iter()
        .any(|a| a.eq_ignore_ascii_case("focus"))
        || !out.is_empty()
    {
        out.insert(0, ComponentAction::new("focus", ActionKind::Focus));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_mac_ax_nodes_to_semantic_snapshot_and_redacts_sensitive_values() {
        let assoc = AccessibilityWindowAssociation {
            platform: AccessibilityPlatform::MacAx,
            surface: "native-1".to_string(),
            pid: Some(42),
            platform_window_id: Some(7),
            title: "Preferences".to_string(),
        };
        let root = AccessibilityNode::new("ax:window", "AXWindow")
            .named("Preferences")
            .children(vec![
                AccessibilityNode::new("ax:ok", "AXButton")
                    .named("OK")
                    .focusable()
                    .actions(["AXPress"]),
                AccessibilityNode::new("ax:enabled", "AXCheckBox")
                    .named("Enabled")
                    .focusable(),
                AccessibilityNode::new("ax:password", "AXTextField")
                    .named("Password")
                    .valued("secret")
                    .focusable()
                    .sensitive(),
                AccessibilityNode::new("ax:volume", "AXSlider")
                    .named("Volume")
                    .valued("0.75"),
            ]);
        let snapshot = accessibility_snapshot_from_tree(&assoc, &root);
        assert_eq!(snapshot.surface, "native-1");
        assert_eq!(snapshot.root.role, ComponentRole::Group);
        assert_eq!(snapshot.root.children.len(), 4);
        assert_eq!(snapshot.root.children[0].role, ComponentRole::Button);
        assert!(snapshot.root.children[0]
            .actions
            .iter()
            .any(|a| a.id == "activate"));
        assert_eq!(snapshot.root.children[1].role, ComponentRole::Checkbox);
        assert!(snapshot.root.children[2].state.sensitive);
        assert!(snapshot.root.children[2].value.is_none());
        assert_eq!(
            snapshot.root.children[3].value,
            Some(ComponentValue::Number(0.75))
        );
    }

    #[test]
    fn maps_linux_atspi_nodes_to_semantic_snapshot_and_degrades_cleanly() {
        let assoc = AccessibilityWindowAssociation {
            platform: AccessibilityPlatform::LinuxAtSpi,
            surface: "native-2".to_string(),
            pid: Some(99),
            platform_window_id: Some(0x1200007),
            title: "Settings".to_string(),
        };
        let root = AccessibilityNode::new("atspi:window", "frame")
            .named("Settings")
            .children(vec![
                AccessibilityNode::new("atspi:apply", "push button")
                    .named("Apply")
                    .focusable()
                    .actions(["click"]),
                AccessibilityNode::new("atspi:username", "text")
                    .named("Username")
                    .valued("ada")
                    .focusable()
                    .actions(["focus"]),
                AccessibilityNode::new("atspi:choice", "combo box")
                    .named("Profile")
                    .valued("Developer")
                    .children(vec![
                        AccessibilityNode::new("atspi:choice.dev", "list item")
                            .named("Developer")
                            .focusable(),
                        AccessibilityNode::new("atspi:choice.ops", "list item")
                            .named("Operator")
                            .focusable(),
                    ]),
                AccessibilityNode::new("atspi:progress", "progress bar").valued("0.5"),
            ]);
        let snapshot = accessibility_snapshot_from_tree(&assoc, &root);
        assert_eq!(snapshot.surface, "native-2");
        assert_eq!(snapshot.root.role, ComponentRole::Group);
        assert_eq!(snapshot.root.children[0].role, ComponentRole::Button);
        assert!(snapshot.root.children[0]
            .actions
            .iter()
            .any(|action| action.id == "activate"));
        assert_eq!(snapshot.root.children[1].role, ComponentRole::TextInput);
        assert_eq!(
            snapshot.root.children[1].value,
            Some(ComponentValue::Text("ada".to_string()))
        );
        assert_eq!(snapshot.root.children[2].role, ComponentRole::SelectList);
        assert_eq!(snapshot.root.children[2].children.len(), 2);
        assert_eq!(snapshot.root.children[3].role, ComponentRole::Progress);
        assert_eq!(
            snapshot.root.children[3].value,
            Some(ComponentValue::Number(0.5))
        );

        let diag = AccessibilityDiagnostics::linux_atspi_unavailable("AT-SPI bus unavailable");
        assert_eq!(diag.platform, AccessibilityPlatform::LinuxAtSpi);
        assert!(!diag.available);
        assert!(diag.reason.unwrap().contains("AT-SPI"));
    }

    #[test]
    fn mac_ax_unknown_diagnostic_reports_permission_requirement() {
        let diag = AccessibilityDiagnostics::mac_ax_unknown();
        assert_eq!(diag.platform, AccessibilityPlatform::MacAx);
        assert!(!diag.available);
        assert!(diag.reason.unwrap().contains("permission"));
    }
}
