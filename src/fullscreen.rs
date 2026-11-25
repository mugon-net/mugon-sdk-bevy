//! A fullscreen toggle for UI entities, without any UI of its own.

use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use bevy_ui_widgets::Activate;

use crate::bindings;

/// Marks a UI entity that toggles the game frame's fullscreen when activated (`window.requestFullscreen()`
/// on the frame's own document, not the parent page). Put it on any entity that emits
/// [`Activate`](bevy_ui_widgets::Activate): a `bevy_ui_widgets::Button`, a themed button from
/// `mugon-sdk-ui-bevy`, or your own widget. Its size, position and look are up to the game.
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct MugonFullscreenToggle;

/// Runs [`MugonFullscreenToggle`]. Does not spawn anything.
pub struct MugonFullscreenPlugin;

impl Plugin for MugonFullscreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(toggle_fullscreen);
    }
}

fn toggle_fullscreen(activate: On<Activate>, toggles: Query<(), With<MugonFullscreenToggle>>) {
    if toggles.contains(activate.entity) {
        bindings::context().toggle_fullscreen();
    }
}
