//! Reports asset loading to the platform's loading overlay: [`MugonLoadingPlugin`] turns the
//! assets a game tracks with [`MugonLoading::track`] into `ctx.progress` updates, and tells the
//! platform the game is playable once all of them are loaded.

use crate::Bridge;
use bevy_app::{App, Plugin, Update};
use bevy_asset::{
    AssetServer, DependencyLoadState, LoadState, RecursiveDependencyLoadState, UntypedHandle,
};
use bevy_ecs::prelude::*;
use bevy_ecs::system::NonSend;

/// The assets whose loading holds the platform's loading overlay open.
///
/// Track everything the game needs before it is playable (typically in a `Startup` system):
/// once every tracked asset (and its dependencies) has finished loading or failed, the overlay
/// is closed, and assets tracked afterwards are not waited for. Loading nothing at all is the
/// game's own business — call [`Bridge::loaded`] directly instead.
#[derive(Resource, Default)]
pub struct MugonLoading {
    tracked: Vec<UntypedHandle>,
    last_reported: Option<(usize, usize)>,
    finished: bool,
}

impl MugonLoading {
    /// Holds the overlay open until `handle` has loaded. Keeps the handle (and so the asset)
    /// alive for the rest of the game.
    pub fn track(&mut self, handle: impl Into<UntypedHandle>) -> &mut Self {
        self.tracked.push(handle.into());
        self
    }
}

pub struct MugonLoadingPlugin;

impl Plugin for MugonLoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MugonLoading>();
        app.add_systems(Update, report_loading);
    }
}

fn report_loading(
    asset_server: Res<AssetServer>,
    mut loading: ResMut<MugonLoading>,
    bridge: NonSend<Bridge>,
) {
    if loading.finished || loading.tracked.is_empty() {
        return;
    }

    let mut finished = 0;
    let mut failed = 0;
    for handle in &loading.tracked {
        match asset_server.get_load_states(handle.id()) {
            Some((LoadState::Failed(_), _, _))
            | Some((_, _, RecursiveDependencyLoadState::Failed(_))) => {
                finished += 1;
                failed += 1;
            }
            Some((
                LoadState::Loaded,
                DependencyLoadState::Loaded,
                RecursiveDependencyLoadState::Loaded,
            )) => finished += 1,
            _ => {}
        }
    }
    let total = loading.tracked.len();

    if loading.last_reported != Some((finished, total)) {
        loading.last_reported = Some((finished, total));
        let label = if failed == 0 {
            format!("{finished}/{total} assets")
        } else {
            format!("{finished}/{total} assets ({failed} failed)")
        };
        bridge.set_progress(finished as f32 / total as f32, Some(&label));
    }
    if finished == total {
        loading.finished = true;
        bridge.loaded();
    }
}
