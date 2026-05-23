//! First-party affordance galleries for examples, docs, and smoke tests.
//!
//! The gallery helpers intentionally return regular affordance components and
//! primitive kittui scenes. They are examples of composition, not new core
//! widgets.

use kittui::{CellSize, Scene};

use crate::controls::{
    button, checkbox, menu, progress, radio_group, select_list, slider, split_pane, tabs,
    text_area, text_input, ControlComponent, ControlOption, ControlState,
};

/// Build a representative first-party control palette.
pub fn control_gallery() -> Vec<ControlComponent> {
    let theme = vec![
        ControlOption::new("theme.light", "Light"),
        ControlOption::new("theme.dark", "Dark").selected(true),
        ControlOption::new("theme.system", "System"),
    ];
    let profiles = vec![
        ControlOption::new("profile.dev", "Developer").selected(true),
        ControlOption::new("profile.ops", "Operator"),
        ControlOption::new("profile.guest", "Guest").disabled(true),
    ];
    let menu_items = vec![
        ControlOption::new("file.new", "New"),
        ControlOption::new("file.open", "Open"),
        ControlOption::new("file.quit", "Quit"),
    ];
    let tabs_items = vec![
        ControlOption::new("tab.general", "General").selected(true),
        ControlOption::new("tab.advanced", "Advanced"),
    ];

    vec![
        button("save", "Save", 24).state(ControlState::default().focused(true)),
        checkbox("notifications", "Notifications", true, 32),
        radio_group("theme", "Theme", theme, 32),
        text_input("name", "Display name", "Ada", 32),
        text_area("bio", "Bio", "Loves terminal-native UI", 32, 5),
        select_list("profile", "Profile", profiles, 32),
        menu("file", "File", menu_items, 32),
        slider("volume", "Volume", 0.6, 32),
        progress("sync", "Sync", 0.72, 32),
        tabs("sections", "Sections", tabs_items, 32),
        split_pane("preview", "Preview split", 32, 6),
    ]
}

/// Lower the control gallery to primitive kittui scenes.
pub fn control_gallery_scenes(cell_size: CellSize) -> Vec<Scene> {
    control_gallery()
        .into_iter()
        .map(|control| control.to_scene(cell_size))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::ControlKind;

    #[test]
    fn control_gallery_covers_first_party_controls() {
        let gallery = control_gallery();
        let kinds = gallery
            .iter()
            .map(|control| control.kind)
            .collect::<Vec<_>>();
        assert!(kinds.contains(&ControlKind::Button));
        assert!(kinds.contains(&ControlKind::Checkbox));
        assert!(kinds.contains(&ControlKind::RadioGroup));
        assert!(kinds.contains(&ControlKind::TextInput));
        assert!(kinds.contains(&ControlKind::TextArea));
        assert!(kinds.contains(&ControlKind::SelectList));
        assert!(kinds.contains(&ControlKind::Menu));
        assert!(kinds.contains(&ControlKind::Slider));
        assert!(kinds.contains(&ControlKind::Progress));
        assert!(kinds.contains(&ControlKind::Tabs));
        assert!(kinds.contains(&ControlKind::SplitPane));
    }

    #[test]
    fn control_gallery_lowers_to_primitive_scenes() {
        let scenes = control_gallery_scenes(CellSize::new(8, 16));
        assert_eq!(scenes.len(), control_gallery().len());
        assert!(scenes.iter().all(|scene| !scene.layers.is_empty()));
        assert!(scenes
            .iter()
            .flat_map(|scene| scene.layers.iter())
            .any(|layer| layer.label.as_deref() == Some("control_progress_fill")));
    }
}
