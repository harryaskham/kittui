//! Print a tiny summary of the first-party kittui-affordances control gallery.

use kittui::CellSize;
use kittui_affordances::{control_gallery, control_gallery_scenes};

fn main() {
    let controls = control_gallery();
    let scenes = control_gallery_scenes(CellSize::default());
    println!("kittui-affordances control gallery");
    for (control, scene) in controls.iter().zip(scenes.iter()) {
        println!(
            "- {:?}: id={} label={:?} size={}x{} layers={}",
            control.kind,
            control.id,
            control.label,
            scene.footprint.cols,
            scene.footprint.rows,
            scene.layers.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_example_has_matching_controls_and_scenes() {
        let controls = control_gallery();
        let scenes = control_gallery_scenes(CellSize::new(8, 16));
        assert_eq!(controls.len(), scenes.len());
        assert!(controls.len() >= 10);
    }
}
