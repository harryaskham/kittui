//! First-party form and control affordances.
//!
//! These builders intentionally live above `kittui-core`: they carry semantic
//! control state for kittwm/SDK/component surfaces while still being able to
//! lower to ordinary kittui primitive scenes for renderers and tests.

use kittui::{
    scene, Animation, CellRect, CellSize, Corners, Layer, Node, Paint, PxRect, Rgba, Scene, Stroke,
    STANDARD_ANIMATION_FPS, STANDARD_ANIMATION_FRAMES,
};
use ratakittui::{Background, Border, Chrome, Padding};

use crate::palette::{Palette, Tone};

/// High-level control kind.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ControlKind {
    /// Momentary action button.
    Button,
    /// Boolean checkbox.
    Checkbox,
    /// Single radio option.
    Radio,
    /// A group of mutually exclusive radio options.
    RadioGroup,
    /// Single-line text input.
    TextInput,
    /// Multi-line text area.
    TextArea,
    /// Select/list control.
    SelectList,
    /// Menu or command list.
    Menu,
    /// Slider with a numeric range.
    Slider,
    /// Progress bar.
    Progress,
    /// Tab strip.
    Tabs,
    /// Two-pane split container.
    SplitPane,
}

impl ControlKind {
    /// Stable snake-case label for scene layer names.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::Checkbox => "checkbox",
            Self::Radio => "radio",
            Self::RadioGroup => "radio_group",
            Self::TextInput => "text_input",
            Self::TextArea => "text_area",
            Self::SelectList => "select_list",
            Self::Menu => "menu",
            Self::Slider => "slider",
            Self::Progress => "progress",
            Self::Tabs => "tabs",
            Self::SplitPane => "split_pane",
        }
    }
}

/// Shared visual/semantic state for controls.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ControlState {
    /// Control currently has keyboard focus.
    pub focused: bool,
    /// Control is disabled and should not accept actions.
    pub disabled: bool,
    /// Control is currently active/pressed.
    pub active: bool,
    /// Control or option is selected.
    pub selected: bool,
    /// Checkbox/radio is checked.
    pub checked: bool,
}

impl ControlState {
    /// Mark the state focused.
    pub const fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Mark the state disabled.
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Mark the state active/pressed.
    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Mark the state selected.
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Mark the state checked.
    pub const fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
}

/// Option metadata for radio groups, selects, lists, and tabs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlOption {
    /// Stable option id for semantic actions.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Whether this option is selected.
    pub selected: bool,
    /// Whether this option is disabled.
    pub disabled: bool,
}

impl ControlOption {
    /// Build an enabled, unselected option.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            selected: false,
            disabled: false,
        }
    }

    /// Set selected state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Set disabled state.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Kitty-native animation options for control scenes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ControlAnimation {
    /// Frames per second.
    pub fps: u16,
    /// Frames in one seamless loop.
    pub frames: u16,
}

impl Default for ControlAnimation {
    fn default() -> Self {
        Self {
            fps: STANDARD_ANIMATION_FPS,
            frames: STANDARD_ANIMATION_FRAMES,
        }
    }
}

impl ControlAnimation {
    /// Convert to the kittui core animation descriptor.
    pub fn to_animation(self) -> Animation {
        Animation::pulse_fps(self.frames, self.fps)
    }
}

/// Reusable high-level form/control component.
#[derive(Clone, Debug)]
pub struct ControlComponent {
    /// Semantic kind.
    pub kind: ControlKind,
    /// Stable component id for semantic action routing.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Optional string value.
    pub value: Option<String>,
    /// Suggested width in terminal cells.
    pub width_cells: u16,
    /// Suggested height in terminal cells.
    pub height_cells: u16,
    /// Shared state flags.
    pub state: ControlState,
    /// Child options for grouped controls.
    pub options: Vec<ControlOption>,
    /// Optional numeric value for progress/slider-like controls, normalized to `[0, 1]`.
    pub numeric_value: Option<f32>,
    /// Visual chrome for ratakittui consumers.
    pub chrome: Chrome,
    /// Optional kitty-native animation descriptor for rendered scenes.
    pub animation: Option<ControlAnimation>,
}

impl ControlComponent {
    /// Build a button.
    pub fn button(id: impl Into<String>, label: impl Into<String>, width_cells: u16) -> Self {
        Self::base(
            ControlKind::Button,
            id,
            label,
            width_cells.max(6),
            3,
            Tone::Assistant,
        )
    }

    /// Build a checkbox.
    pub fn checkbox(
        id: impl Into<String>,
        label: impl Into<String>,
        checked: bool,
        width_cells: u16,
    ) -> Self {
        Self::base(
            ControlKind::Checkbox,
            id,
            label,
            width_cells.max(6),
            1,
            Tone::Tool,
        )
        .state(ControlState::default().checked(checked))
    }

    /// Build a radio option.
    pub fn radio(
        id: impl Into<String>,
        label: impl Into<String>,
        selected: bool,
        width_cells: u16,
    ) -> Self {
        Self::base(
            ControlKind::Radio,
            id,
            label,
            width_cells.max(6),
            1,
            Tone::Tool,
        )
        .state(ControlState::default().checked(selected).selected(selected))
    }

    /// Build a radio group.
    pub fn radio_group(
        id: impl Into<String>,
        label: impl Into<String>,
        options: Vec<ControlOption>,
        width_cells: u16,
    ) -> Self {
        let height_cells = options.len().saturating_add(2).min(u16::MAX as usize) as u16;
        Self::base(
            ControlKind::RadioGroup,
            id,
            label,
            width_cells.max(10),
            height_cells.max(3),
            Tone::Tool,
        )
        .options(options)
    }

    /// Build a single-line text input.
    pub fn text_input(
        id: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
        width_cells: u16,
    ) -> Self {
        Self::base(
            ControlKind::TextInput,
            id,
            label,
            width_cells.max(8),
            3,
            Tone::User,
        )
        .value(value.into())
    }

    /// Build a multi-line text area.
    pub fn text_area(
        id: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
        width_cells: u16,
        height_cells: u16,
    ) -> Self {
        Self::base(
            ControlKind::TextArea,
            id,
            label,
            width_cells.max(8),
            height_cells.max(3),
            Tone::User,
        )
        .value(value.into())
    }

    /// Build a select/list control.
    pub fn select_list(
        id: impl Into<String>,
        label: impl Into<String>,
        options: Vec<ControlOption>,
        width_cells: u16,
    ) -> Self {
        let visible = options.len().clamp(1, 6) as u16;
        Self::base(
            ControlKind::SelectList,
            id,
            label,
            width_cells.max(10),
            visible.saturating_add(2),
            Tone::User,
        )
        .options(options)
    }

    /// Build a menu or command list.
    pub fn menu(
        id: impl Into<String>,
        label: impl Into<String>,
        items: Vec<ControlOption>,
        width_cells: u16,
    ) -> Self {
        let visible = items.len().clamp(1, 8) as u16;
        Self::base(
            ControlKind::Menu,
            id,
            label,
            width_cells.max(10),
            visible.saturating_add(2),
            Tone::Tool,
        )
        .options(items)
    }

    /// Build a slider. `value` is clamped to `[0, 1]`.
    pub fn slider(
        id: impl Into<String>,
        label: impl Into<String>,
        value: f32,
        width_cells: u16,
    ) -> Self {
        let clamped = value.clamp(0.0, 1.0);
        Self::base(
            ControlKind::Slider,
            id,
            label,
            width_cells.max(8),
            1,
            Tone::Assistant,
        )
        .numeric_value(clamped)
    }

    /// Build a progress bar. `value` is clamped to `[0, 1]`.
    pub fn progress(
        id: impl Into<String>,
        label: impl Into<String>,
        value: f32,
        width_cells: u16,
    ) -> Self {
        let clamped = value.clamp(0.0, 1.0);
        Self::base(
            ControlKind::Progress,
            id,
            label,
            width_cells.max(8),
            1,
            Tone::Assistant,
        )
        .numeric_value(clamped)
    }

    /// Build a tab strip.
    pub fn tabs(
        id: impl Into<String>,
        label: impl Into<String>,
        tabs: Vec<ControlOption>,
        width_cells: u16,
    ) -> Self {
        Self::base(
            ControlKind::Tabs,
            id,
            label,
            width_cells.max(10),
            2,
            Tone::Assistant,
        )
        .options(tabs)
    }

    /// Build a split-pane affordance.
    pub fn split_pane(
        id: impl Into<String>,
        label: impl Into<String>,
        width_cells: u16,
        height_cells: u16,
    ) -> Self {
        Self::base(
            ControlKind::SplitPane,
            id,
            label,
            width_cells.max(12),
            height_cells.max(3),
            Tone::Tool,
        )
    }

    /// Replace state flags.
    pub fn state(mut self, state: ControlState) -> Self {
        self.state = state;
        self.chrome = control_chrome(self.kind, self.state);
        self
    }

    /// Set options.
    pub fn options(mut self, options: Vec<ControlOption>) -> Self {
        self.options = options;
        self
    }

    /// Set string value.
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Set numeric value.
    pub fn numeric_value(mut self, value: f32) -> Self {
        self.numeric_value = Some(value.clamp(0.0, 1.0));
        self
    }

    /// Enable or disable default kitty-native control animation.
    pub fn animated(mut self, animated: bool) -> Self {
        self.animation = animated.then(ControlAnimation::default);
        self
    }

    /// Set explicit kitty-native animation options.
    pub fn animation(mut self, animation: ControlAnimation) -> Self {
        self.animation = Some(animation);
        self
    }

    /// Lower the control to a primitive kittui scene.
    ///
    /// The scene intentionally encodes structural state using primitive rect
    /// layers only. Text layout stays with terminal/ratakittui renderers until
    /// kittui grows first-class text primitives.
    pub fn to_scene(&self, cell_size: CellSize) -> Scene {
        let footprint = CellRect::new(0, 0, self.width_cells, self.height_cells);
        let rect = footprint.to_pixels(cell_size);
        let mut layers = vec![Layer::new(
            "control_background",
            control_rect(
                rect,
                fill_for(self.kind, self.state),
                stroke_for(self.state),
                radius_for(self.kind),
            ),
        )];

        match self.kind {
            ControlKind::Checkbox | ControlKind::Radio => {
                let marker = marker_rect(cell_size, self.kind);
                layers.push(Layer::new(
                    "control_marker",
                    control_rect(
                        marker,
                        marker_fill(self.state),
                        stroke_for(self.state),
                        radius_for(self.kind),
                    ),
                ));
                if self.state.checked || self.state.selected {
                    layers.push(Layer::new(
                        "control_marker_selected",
                        control_rect(
                            inset(marker, 3.0),
                            stroke_for(self.state),
                            stroke_for(self.state),
                            radius_for(self.kind),
                        ),
                    ));
                }
            }
            ControlKind::Slider | ControlKind::Progress => {
                let frac = self.numeric_value.unwrap_or(0.0).clamp(0.0, 1.0);
                let filled =
                    PxRect::new(rect.origin.0, rect.origin.1, rect.width * frac, rect.height);
                layers.push(Layer::new(
                    "control_progress_fill",
                    control_rect(filled, stroke_for(self.state), stroke_for(self.state), 3.0),
                ));
            }
            ControlKind::Tabs => {
                for (idx, option) in self.options.iter().enumerate() {
                    let count = self.options.len().max(1) as f32;
                    let tab_w = rect.width / count;
                    let tab = PxRect::new(
                        rect.origin.0 + (idx as f32 * tab_w),
                        rect.origin.1,
                        tab_w,
                        rect.height,
                    );
                    let selected = ControlState {
                        selected: option.selected,
                        disabled: option.disabled,
                        ..self.state
                    };
                    layers.push(Layer::new(
                        format!("control_tab_{idx}"),
                        control_rect(
                            tab,
                            fill_for(ControlKind::Button, selected),
                            stroke_for(selected),
                            4.0,
                        ),
                    ));
                }
            }
            ControlKind::SplitPane => {
                let divider = PxRect::new(
                    rect.origin.0 + rect.width / 2.0 - 1.0,
                    rect.origin.1,
                    2.0,
                    rect.height,
                );
                layers.push(Layer::new(
                    "control_split_divider",
                    control_rect(divider, stroke_for(self.state), stroke_for(self.state), 0.0),
                ));
            }
            ControlKind::RadioGroup | ControlKind::SelectList | ControlKind::Menu => {
                for (idx, option) in self.options.iter().enumerate() {
                    let row_h = cell_size.height_px.max(1) as f32;
                    let row = PxRect::new(
                        rect.origin.0 + 4.0,
                        rect.origin.1 + row_h * (idx as f32 + 1.0),
                        (rect.width - 8.0).max(1.0),
                        (row_h - 2.0).max(1.0),
                    );
                    let selected = ControlState {
                        selected: option.selected,
                        disabled: option.disabled,
                        ..self.state
                    };
                    layers.push(Layer::new(
                        format!("control_option_{idx}"),
                        control_rect(
                            row,
                            fill_for(ControlKind::Button, selected),
                            stroke_for(selected),
                            3.0,
                        ),
                    ));
                }
            }
            ControlKind::Button | ControlKind::TextInput | ControlKind::TextArea => {}
        }

        if let Some(animation) = self.animation {
            layers.push(Layer::new(
                format!("control_animation_{}", self.kind.as_str()),
                Node::Glow {
                    rect,
                    center_x_frac: 0.5,
                    center_y_frac: 0.35,
                    radius_frac: 2.0,
                    color: stroke_for(self.state),
                    intensity: 0.55,
                },
            ));
            let mut scene = scene::scene(footprint, cell_size, layers);
            scene.animation = Some(animation.to_animation());
            scene
        } else {
            scene::scene(footprint, cell_size, layers)
        }
    }

    fn base(
        kind: ControlKind,
        id: impl Into<String>,
        label: impl Into<String>,
        width_cells: u16,
        height_cells: u16,
        tone: Tone,
    ) -> Self {
        let state = ControlState::default();
        Self {
            kind,
            id: id.into(),
            label: label.into(),
            value: None,
            width_cells,
            height_cells,
            state,
            options: Vec::new(),
            numeric_value: None,
            chrome: control_chrome_tone(kind, state, tone),
            animation: None,
        }
    }
}

/// Convenience button constructor.
pub fn button(
    id: impl Into<String>,
    label: impl Into<String>,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::button(id, label, width_cells)
}

/// Convenience checkbox constructor.
pub fn checkbox(
    id: impl Into<String>,
    label: impl Into<String>,
    checked: bool,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::checkbox(id, label, checked, width_cells)
}

/// Convenience radio constructor.
pub fn radio(
    id: impl Into<String>,
    label: impl Into<String>,
    selected: bool,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::radio(id, label, selected, width_cells)
}

/// Convenience radio-group constructor.
pub fn radio_group(
    id: impl Into<String>,
    label: impl Into<String>,
    options: Vec<ControlOption>,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::radio_group(id, label, options, width_cells)
}

/// Convenience text-input constructor.
pub fn text_input(
    id: impl Into<String>,
    label: impl Into<String>,
    value: impl Into<String>,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::text_input(id, label, value, width_cells)
}

/// Convenience text-area constructor.
pub fn text_area(
    id: impl Into<String>,
    label: impl Into<String>,
    value: impl Into<String>,
    width_cells: u16,
    height_cells: u16,
) -> ControlComponent {
    ControlComponent::text_area(id, label, value, width_cells, height_cells)
}

/// Convenience select/list constructor.
pub fn select_list(
    id: impl Into<String>,
    label: impl Into<String>,
    options: Vec<ControlOption>,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::select_list(id, label, options, width_cells)
}

/// Convenience menu constructor.
pub fn menu(
    id: impl Into<String>,
    label: impl Into<String>,
    items: Vec<ControlOption>,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::menu(id, label, items, width_cells)
}

/// Convenience slider constructor.
pub fn slider(
    id: impl Into<String>,
    label: impl Into<String>,
    value: f32,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::slider(id, label, value, width_cells)
}

/// Convenience progress constructor.
pub fn progress(
    id: impl Into<String>,
    label: impl Into<String>,
    value: f32,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::progress(id, label, value, width_cells)
}

/// Convenience tabs constructor.
pub fn tabs(
    id: impl Into<String>,
    label: impl Into<String>,
    tabs: Vec<ControlOption>,
    width_cells: u16,
) -> ControlComponent {
    ControlComponent::tabs(id, label, tabs, width_cells)
}

/// Convenience split-pane constructor.
pub fn split_pane(
    id: impl Into<String>,
    label: impl Into<String>,
    width_cells: u16,
    height_cells: u16,
) -> ControlComponent {
    ControlComponent::split_pane(id, label, width_cells, height_cells)
}

fn control_chrome(kind: ControlKind, state: ControlState) -> Chrome {
    let tone = match kind {
        ControlKind::Button | ControlKind::Progress | ControlKind::Slider | ControlKind::Tabs => {
            Tone::Assistant
        }
        ControlKind::TextInput | ControlKind::TextArea | ControlKind::SelectList => Tone::User,
        ControlKind::Checkbox
        | ControlKind::Radio
        | ControlKind::RadioGroup
        | ControlKind::Menu
        | ControlKind::SplitPane => Tone::Tool,
    };
    control_chrome_tone(kind, state, tone)
}

fn control_chrome_tone(_kind: ControlKind, state: ControlState, tone: Tone) -> Chrome {
    let p = Palette::for_tone(tone);
    let border = if state.focused { p.glow } else { p.rail };
    let fill = if state.disabled {
        Rgba::rgba(34, 38, 48, 150)
    } else {
        p.bg_top
    };
    Chrome::default()
        .background(Background::Solid(fill))
        .border(Border::rounded(
            border,
            if state.focused { 2.0 } else { 1.0 },
            5.0,
        ))
        .padding(Padding::trbl(0, 1, 0, 1))
}

fn control_rect(rect: PxRect, fill: Rgba, stroke: Rgba, radius: f32) -> Node {
    Node::Rect {
        rect,
        fill: Paint::Solid { color: fill },
        stroke: Some(Stroke::inside(1.0, Paint::Solid { color: stroke })),
        corners: Corners::uniform(radius),
    }
}

fn fill_for(kind: ControlKind, state: ControlState) -> Rgba {
    if state.disabled {
        return Rgba::rgba(38, 42, 52, 160);
    }
    if state.active || state.selected || state.checked {
        return Rgba::rgba(88, 166, 255, 220);
    }
    match kind {
        ControlKind::TextInput | ControlKind::TextArea => Rgba::rgba(24, 29, 39, 230),
        ControlKind::Progress | ControlKind::Slider => Rgba::rgba(22, 27, 34, 210),
        _ => Rgba::rgba(34, 42, 58, 210),
    }
}

fn stroke_for(state: ControlState) -> Rgba {
    if state.focused {
        Rgba::rgba(140, 210, 255, 255)
    } else if state.disabled {
        Rgba::rgba(96, 100, 112, 180)
    } else {
        Rgba::rgba(122, 162, 247, 220)
    }
}

fn marker_fill(state: ControlState) -> Rgba {
    if state.checked || state.selected {
        Rgba::rgba(140, 210, 255, 255)
    } else {
        Rgba::rgba(12, 16, 24, 220)
    }
}

fn radius_for(kind: ControlKind) -> f32 {
    match kind {
        ControlKind::Radio => 99.0,
        ControlKind::Checkbox | ControlKind::Progress | ControlKind::SplitPane => 2.0,
        _ => 5.0,
    }
}

fn marker_rect(cell_size: CellSize, kind: ControlKind) -> PxRect {
    let size = match kind {
        ControlKind::Radio => cell_size
            .height_px
            .min(cell_size.width_px.saturating_mul(2))
            .max(8) as f32,
        _ => cell_size.height_px.max(8) as f32,
    };
    PxRect::new(2.0, 2.0, size - 4.0, size - 4.0)
}

fn inset(rect: PxRect, amount: f32) -> PxRect {
    PxRect::new(
        rect.origin.0 + amount,
        rect.origin.1 + amount,
        (rect.width - amount * 2.0).max(1.0),
        (rect.height - amount * 2.0).max(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_builders_cover_required_semantics() {
        let opts = vec![
            ControlOption::new("a", "Alpha").selected(true),
            ControlOption::new("b", "Beta"),
        ];
        assert_eq!(button("save", "Save", 12).kind, ControlKind::Button);
        assert_eq!(checkbox("notify", "Notify", true, 20).state.checked, true);
        assert_eq!(radio("r1", "One", true, 12).kind, ControlKind::Radio);
        assert_eq!(
            radio_group("group", "Mode", opts.clone(), 20).options.len(),
            2
        );
        assert_eq!(
            text_input("name", "Name", "Ada", 20).value.as_deref(),
            Some("Ada")
        );
        assert_eq!(
            text_area("body", "Body", "hello", 20, 5).kind,
            ControlKind::TextArea
        );
        assert_eq!(
            select_list("choice", "Choice", opts.clone(), 20).kind,
            ControlKind::SelectList
        );
        assert_eq!(
            menu("menu", "Menu", opts.clone(), 20).kind,
            ControlKind::Menu
        );
        assert_eq!(
            slider("volume", "Volume", -1.0, 20).numeric_value,
            Some(0.0)
        );
        assert_eq!(
            progress("load", "Loading", 2.0, 20).numeric_value,
            Some(1.0)
        );
        assert_eq!(tabs("tabs", "Tabs", opts, 20).kind, ControlKind::Tabs);
        assert_eq!(
            split_pane("split", "Split", 30, 8).kind,
            ControlKind::SplitPane
        );
    }

    #[test]
    fn animated_controls_emit_looping_scene_animation() {
        let scene = button("save", "Save", 12)
            .animated(true)
            .to_scene(CellSize::new(8, 16));
        let animation = scene.animation.as_ref().unwrap();
        assert_eq!(animation.frames, 180);
        assert_eq!(animation.cycle_ms, 3000);
        assert!(animation.curve.closes_loop());
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("control_animation_button")));

        let scene = slider("volume", "Volume", 0.5, 20)
            .animation(ControlAnimation {
                fps: 30,
                frames: 90,
            })
            .to_scene(CellSize::new(8, 16));
        let animation = scene.animation.as_ref().unwrap();
        assert_eq!(animation.frames, 90);
        assert_eq!(animation.cycle_ms, 3000);
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("control_animation_slider")));
    }

    #[test]
    fn controls_lower_to_primitive_scenes() {
        let scene = checkbox("notify", "Notify", true, 20).to_scene(CellSize::new(8, 16));
        assert_eq!(scene.footprint.cols, 20);
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("control_marker_selected")));

        let tabs = tabs(
            "tabs",
            "Tabs",
            vec![
                ControlOption::new("one", "One").selected(true),
                ControlOption::new("two", "Two"),
            ],
            24,
        )
        .to_scene(CellSize::new(8, 16));
        assert!(tabs
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("control_tab_0")));
    }
}
