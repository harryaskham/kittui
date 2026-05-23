//! Semantic component surface model and renderer bridge.
//!
//! This module is intentionally small and synthetic-first. It gives kittwm a
//! native place to represent semantic component trees and lower them through
//! `kittui-affordances` without adding high-level controls to `kittui-core`.

use kittui::{CellRect, CellSize, Layer, Node, PxRect, Scene};
use kittui_affordances::{
    button, checkbox, progress, radio_group, select_list, split_pane, tabs, text_area, text_input,
    ControlComponent, ControlOption, ControlState,
};

/// Stable semantic component identifier.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ComponentId(String);

impl ComponentId {
    /// Create a component id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the raw id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Role of a semantic component node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComponentRole {
    /// Generic grouping/container.
    Group,
    /// Static label.
    Label,
    /// Action button.
    Button,
    /// Checkbox.
    Checkbox,
    /// Radio group.
    RadioGroup,
    /// Single-line text input.
    TextInput,
    /// Multi-line text area.
    TextArea,
    /// Select/list.
    SelectList,
    /// Progress value.
    Progress,
    /// Tabs.
    Tabs,
    /// Split pane.
    SplitPane,
    /// Unknown/custom role with fallback semantics.
    Custom(String),
}

/// Typed semantic value.
#[derive(Clone, Debug, PartialEq)]
pub enum ComponentValue {
    /// Boolean value for checkboxes/toggles.
    Bool(bool),
    /// Text value for inputs.
    Text(String),
    /// Normalized numeric value for progress/slider-like controls.
    Number(f32),
    /// Selected option ids.
    Selection(Vec<String>),
}

/// Semantic node state.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ComponentState {
    /// Node is focused.
    pub focused: bool,
    /// Node can receive focus.
    pub focusable: bool,
    /// Node is disabled.
    pub disabled: bool,
    /// Node is active/pressed.
    pub active: bool,
    /// Node is selected.
    pub selected: bool,
    /// Node is checked.
    pub checked: bool,
}

impl From<ComponentState> for ControlState {
    fn from(state: ComponentState) -> Self {
        Self {
            focused: state.focused,
            disabled: state.disabled,
            active: state.active,
            selected: state.selected,
            checked: state.checked,
        }
    }
}

/// Semantic component node.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentNode {
    /// Stable id.
    pub id: ComponentId,
    /// Component role.
    pub role: ComponentRole,
    /// Human-readable label.
    pub label: Option<String>,
    /// Optional typed value.
    pub value: Option<ComponentValue>,
    /// State flags.
    pub state: ComponentState,
    /// Child nodes.
    pub children: Vec<ComponentNode>,
}

impl ComponentNode {
    /// Build a node with no children.
    pub fn new(id: impl Into<String>, role: ComponentRole) -> Self {
        Self {
            id: ComponentId::new(id),
            role,
            label: None,
            value: None,
            state: ComponentState::default(),
            children: Vec::new(),
        }
    }

    /// Set label.
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set value.
    pub fn valued(mut self, value: ComponentValue) -> Self {
        self.value = Some(value);
        self
    }

    /// Set state.
    pub fn state(mut self, state: ComponentState) -> Self {
        self.state = state;
        self
    }

    /// Set children.
    pub fn children(mut self, children: Vec<ComponentNode>) -> Self {
        self.children = children;
        self
    }
}

/// Snapshot of one semantic surface revision.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticSurfaceSnapshot {
    /// Schema version.
    pub schema_version: u32,
    /// Monotonic semantic revision.
    pub revision: u64,
    /// Root component.
    pub root: ComponentNode,
    /// Focused component id, if any.
    pub focus: Option<ComponentId>,
}

impl SemanticSurfaceSnapshot {
    /// Build a v1 snapshot.
    pub fn new(revision: u64, root: ComponentNode) -> Self {
        Self {
            schema_version: 1,
            revision,
            root,
            focus: None,
        }
    }

    /// Set focused component.
    pub fn focused(mut self, id: impl Into<String>) -> Self {
        self.focus = Some(ComponentId::new(id));
        self
    }
}

/// Render a semantic snapshot to a kittui primitive scene using shared
/// `kittui-affordances` control builders.
pub fn render_semantic_surface(snapshot: &SemanticSurfaceSnapshot, cell_size: CellSize) -> Scene {
    let controls = collect_controls(&snapshot.root);
    let width_cells = controls
        .iter()
        .map(|control| control.width_cells)
        .max()
        .unwrap_or(20)
        .max(1);
    let height_cells = controls
        .iter()
        .map(|control| control.height_cells.saturating_add(1))
        .sum::<u16>()
        .saturating_sub(1)
        .max(1);
    let footprint = CellRect::new(0, 0, width_cells, height_cells);
    let mut layers = Vec::new();
    let mut y_cells = 0u16;
    for (index, control) in controls.iter().enumerate() {
        let scene = control.to_scene(cell_size);
        let y_px = y_cells as f32 * cell_size.height_px as f32;
        for layer in scene.layers {
            layers.push(translate_layer(layer, 0.0, y_px, index));
        }
        y_cells = y_cells
            .saturating_add(control.height_cells)
            .saturating_add(1);
    }

    Scene {
        footprint,
        cell_size,
        layers,
        animation: None,
    }
}

fn collect_controls(root: &ComponentNode) -> Vec<ControlComponent> {
    let mut out = Vec::new();
    collect_controls_into(root, &mut out);
    if out.is_empty() {
        out.push(button(
            root.id.as_str(),
            root.label.as_deref().unwrap_or("semantic surface"),
            24,
        ));
    }
    out
}

fn collect_controls_into(node: &ComponentNode, out: &mut Vec<ControlComponent>) {
    if let Some(control) = node_to_control(node) {
        out.push(control);
    }
    for child in &node.children {
        collect_controls_into(child, out);
    }
}

fn node_to_control(node: &ComponentNode) -> Option<ControlComponent> {
    let id = node.id.as_str();
    let label = node.label.as_deref().unwrap_or(id);
    let mut state: ControlState = node.state.into();
    let width = 32;
    let control = match node.role {
        ComponentRole::Button => button(id, label, width).state(state),
        ComponentRole::Checkbox => {
            state.checked = bool_value(node).unwrap_or(node.state.checked);
            checkbox(id, label, state.checked, width).state(state)
        }
        ComponentRole::RadioGroup => {
            radio_group(id, label, option_children(node), width).state(state)
        }
        ComponentRole::TextInput => {
            text_input(id, label, text_value(node).unwrap_or_default(), width).state(state)
        }
        ComponentRole::TextArea => {
            text_area(id, label, text_value(node).unwrap_or_default(), width, 5).state(state)
        }
        ComponentRole::SelectList => {
            select_list(id, label, option_children(node), width).state(state)
        }
        ComponentRole::Progress => {
            progress(id, label, number_value(node).unwrap_or(0.0), width).state(state)
        }
        ComponentRole::Tabs => tabs(id, label, option_children(node), width).state(state),
        ComponentRole::SplitPane => split_pane(id, label, width, 6).state(state),
        ComponentRole::Group | ComponentRole::Label | ComponentRole::Custom(_) => return None,
    };
    Some(control)
}

fn option_children(node: &ComponentNode) -> Vec<ControlOption> {
    let selected = selection_value(node);
    node.children
        .iter()
        .map(|child| {
            let id = child.id.as_str().to_string();
            let is_selected = child.state.selected || selected.iter().any(|s| s == &id);
            ControlOption::new(id, child.label.as_deref().unwrap_or(child.id.as_str()))
                .selected(is_selected)
                .disabled(child.state.disabled)
        })
        .collect()
}

fn bool_value(node: &ComponentNode) -> Option<bool> {
    match node.value.as_ref()? {
        ComponentValue::Bool(v) => Some(*v),
        _ => None,
    }
}

fn text_value(node: &ComponentNode) -> Option<String> {
    match node.value.as_ref()? {
        ComponentValue::Text(v) => Some(v.clone()),
        _ => None,
    }
}

fn number_value(node: &ComponentNode) -> Option<f32> {
    match node.value.as_ref()? {
        ComponentValue::Number(v) => Some(*v),
        _ => None,
    }
}

fn selection_value(node: &ComponentNode) -> Vec<String> {
    match node.value.as_ref() {
        Some(ComponentValue::Selection(v)) => v.clone(),
        _ => Vec::new(),
    }
}

fn translate_layer(mut layer: Layer, dx: f32, dy: f32, index: usize) -> Layer {
    layer.label = layer.label.map(|label| format!("semantic_{index}_{label}"));
    layer.root = translate_node(layer.root, dx, dy);
    layer
}

fn translate_node(node: Node, dx: f32, dy: f32) -> Node {
    match node {
        Node::Rect {
            rect,
            fill,
            stroke,
            corners,
        } => Node::Rect {
            rect: translate_rect(rect, dx, dy),
            fill,
            stroke,
            corners,
        },
        Node::Gradient {
            rect,
            stops,
            direction,
        } => Node::Gradient {
            rect: translate_rect(rect, dx, dy),
            stops,
            direction,
        },
        Node::Glow {
            rect,
            center_x_frac,
            center_y_frac,
            radius_frac,
            color,
            intensity,
        } => Node::Glow {
            rect: translate_rect(rect, dx, dy),
            center_x_frac,
            center_y_frac,
            radius_frac,
            color,
            intensity,
        },
        Node::Scanlines {
            rect,
            alpha,
            period_px,
        } => Node::Scanlines {
            rect: translate_rect(rect, dx, dy),
            alpha,
            period_px,
        },
        Node::Image {
            rect,
            src,
            fit,
            tint,
        } => Node::Image {
            rect: translate_rect(rect, dx, dy),
            src,
            fit,
            tint,
        },
        Node::Group { opacity, children } => Node::Group {
            opacity,
            children: children
                .into_iter()
                .map(|child| translate_node(child, dx, dy))
                .collect(),
        },
        Node::Composite { mode, children } => Node::Composite {
            mode,
            children: children
                .into_iter()
                .map(|child| translate_node(child, dx, dy))
                .collect(),
        },
        Node::Mask { mask, child } => Node::Mask {
            mask: Box::new(translate_node(*mask, dx, dy)),
            child: Box::new(translate_node(*child, dx, dy)),
        },
        Node::Clip { rect, child } => Node::Clip {
            rect: translate_rect(rect, dx, dy),
            child: Box::new(translate_node(*child, dx, dy)),
        },
        Node::Shader {
            rect,
            source,
            uniforms,
        } => Node::Shader {
            rect: translate_rect(rect, dx, dy),
            source,
            uniforms,
        },
    }
}

fn translate_rect(rect: PxRect, dx: f32, dy: f32) -> PxRect {
    PxRect::new(
        rect.origin.0 + dx,
        rect.origin.1 + dy,
        rect.width,
        rect.height,
    )
}

/// Build a small synthetic semantic settings surface for tests/examples.
pub fn synthetic_settings_surface() -> SemanticSurfaceSnapshot {
    let radio_options = vec![
        ComponentNode::new("theme.light", ComponentRole::Label).labeled("Light"),
        ComponentNode::new("theme.dark", ComponentRole::Label)
            .labeled("Dark")
            .state(ComponentState {
                selected: true,
                ..ComponentState::default()
            }),
    ];
    let tabs_children = vec![
        ComponentNode::new("tab.general", ComponentRole::Label)
            .labeled("General")
            .state(ComponentState {
                selected: true,
                ..ComponentState::default()
            }),
        ComponentNode::new("tab.advanced", ComponentRole::Label).labeled("Advanced"),
    ];
    SemanticSurfaceSnapshot::new(
        1,
        ComponentNode::new("settings", ComponentRole::Group)
            .labeled("Settings")
            .children(vec![
                ComponentNode::new("tabs", ComponentRole::Tabs)
                    .labeled("Tabs")
                    .children(tabs_children),
                ComponentNode::new("name", ComponentRole::TextInput)
                    .labeled("Name")
                    .valued(ComponentValue::Text("Ada".to_string()))
                    .state(ComponentState {
                        focused: true,
                        focusable: true,
                        ..ComponentState::default()
                    }),
                ComponentNode::new("notify", ComponentRole::Checkbox)
                    .labeled("Notifications")
                    .valued(ComponentValue::Bool(true)),
                ComponentNode::new("theme", ComponentRole::RadioGroup)
                    .labeled("Theme")
                    .children(radio_options),
                ComponentNode::new("choice", ComponentRole::SelectList)
                    .labeled("Choice")
                    .valued(ComponentValue::Selection(vec!["choice.two".to_string()]))
                    .children(vec![
                        ComponentNode::new("choice.one", ComponentRole::Label).labeled("One"),
                        ComponentNode::new("choice.two", ComponentRole::Label).labeled("Two"),
                    ]),
                ComponentNode::new("progress", ComponentRole::Progress)
                    .labeled("Progress")
                    .valued(ComponentValue::Number(0.66)),
                ComponentNode::new("split", ComponentRole::SplitPane).labeled("Split"),
                ComponentNode::new("save", ComponentRole::Button).labeled("Save"),
            ]),
    )
    .focused("name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittui_affordances::ControlKind;

    #[test]
    fn synthetic_semantic_surface_renders_through_affordance_controls() {
        let snapshot = synthetic_settings_surface();
        let scene = render_semantic_surface(&snapshot, CellSize::new(8, 16));
        assert_eq!(scene.cell_size, CellSize::new(8, 16));
        assert!(scene.footprint.cols >= 32);
        assert!(scene.footprint.rows > 8);
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("semantic_0_control_background")));
        assert!(scene.layers.iter().any(|layer| {
            layer
                .label
                .as_deref()
                .map(|label| label.contains("control_marker_selected"))
                .unwrap_or(false)
        }));
    }

    #[test]
    fn select_value_marks_option_selected() {
        let surface = SemanticSurfaceSnapshot::new(
            1,
            ComponentNode::new("choice", ComponentRole::SelectList)
                .labeled("Choice")
                .valued(ComponentValue::Selection(vec!["b".to_string()]))
                .children(vec![
                    ComponentNode::new("a", ComponentRole::Label).labeled("A"),
                    ComponentNode::new("b", ComponentRole::Label).labeled("B"),
                ]),
        );
        let controls = collect_controls(&surface.root);
        assert_eq!(controls.len(), 1);
        assert_eq!(controls[0].kind, ControlKind::SelectList);
        assert!(controls[0].options[1].selected);
    }

    #[test]
    fn fallback_custom_surface_renders_generic_button() {
        let surface = SemanticSurfaceSnapshot::new(
            1,
            ComponentNode::new("custom", ComponentRole::Custom("vendor.widget".to_string()))
                .labeled("Custom widget"),
        );
        let scene = render_semantic_surface(&surface, CellSize::new(8, 16));
        assert_eq!(scene.footprint.rows, 3);
        assert!(!scene.layers.is_empty());
    }
}
