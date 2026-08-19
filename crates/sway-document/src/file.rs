//! Open and save, by path. Spec M6-8.
//!
//! Deliberately not through the `AssetServer`: asset paths resolve against
//! the `assets/` root, so a dialog-picked absolute path cannot round-trip
//! through it.

use std::path::{Path, PathBuf};

use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;

use crate::apply::apply;
use crate::doc::parse;
use crate::emit::{to_document, to_ron};

/// The file the editor is currently editing. `None` until the first Save As.
#[derive(Resource, Default)]
pub struct CurrentDocument {
    pub path: Option<PathBuf>,
}

pub fn save_to_path(world: &mut World, path: &Path) -> Result<(), String> {
    let doc = to_document(world);
    let text = to_ron(&doc).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    world.insert_resource(CurrentDocument {
        path: Some(path.to_path_buf()),
    });
    Ok(())
}

pub fn open_from_path(world: &mut World, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc = parse(&text).map_err(|e| e.to_string())?;
    let diagnostics = apply(world, &doc);
    world.insert_resource(diagnostics);
    world.insert_resource(CurrentDocument {
        path: Some(path.to_path_buf()),
    });
    // The watcher on any previously-loaded asset path stops mattering.
    if let Some(mut handle) = world.get_resource_mut::<crate::asset::ProjectHandle>() {
        handle.0 = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;
    use bevy_math::Vec2;
    use sway_graph::EditorPos;

    fn file_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy_asset::AssetPlugin::default())
            .add_plugins(crate::ProjectPlugin);
        sway_graph::register_authorable::<EditorPos>(&mut app, "EditorPos");
        app
    }

    #[test]
    fn save_then_open_reproduces_the_world() {
        let dir = std::env::temp_dir().join("sway-m6-save-open");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("round.sway.ron");

        let mut app = file_app();
        app.world_mut().spawn(EditorPos(Vec2::new(5.0, 6.0)));
        app.update();
        save_to_path(app.world_mut(), &path).expect("saves");

        let mut reopened = file_app();
        open_from_path(reopened.world_mut(), &path).expect("opens");

        let positions: Vec<Vec2> = reopened
            .world_mut()
            .query::<&EditorPos>()
            .iter(reopened.world())
            .map(|p| p.0)
            .collect();
        assert_eq!(positions, vec![Vec2::new(5.0, 6.0)]);
    }

    #[test]
    fn saving_records_the_path_for_a_later_plain_save() {
        let dir = std::env::temp_dir().join("sway-m6-save-path");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.sway.ron");

        let mut app = file_app();
        save_to_path(app.world_mut(), &path).expect("saves");

        assert_eq!(
            app.world().resource::<CurrentDocument>().path.as_deref(),
            Some(path.as_path())
        );
    }
}
