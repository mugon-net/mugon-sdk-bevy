//! Static trimesh colliders for the models of exported scene entities (`avian` feature).
//!
//! Tag a scene entity that carries a `WorldAssetRoot` with [`StaticMeshCollider`] and add
//! [`StaticMeshColliderPlugin`]: once the model has loaded, every mesh below it gets an avian3d
//! trimesh [`Collider`] on a [`RigidBody::Static`].
//!
//! Avian derives a collider's scale from the hierarchy above it, starting at the root entity.
//! A root without a `Transform` makes it skip the whole subtree, leaving colliders unscaled.
//! Scenes exported by the editor always carry transforms; [`on_model_ready`] warns if an
//! ancestor lacks one.

use avian3d::prelude::{Collider, RigidBody};
use bevy_app::{App, Plugin};
use bevy_asset::Assets;
use bevy_ecs::prelude::*;
use bevy_log::warn;
use bevy_mesh::{Mesh, Mesh3d};
use bevy_transform::components::Transform;
use bevy_world_serialization::WorldInstanceReady;

/// Marks a model root (the entity with the `WorldAssetRoot`) whose meshes get colliders.
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct StaticMeshCollider;

/// Marks a mesh entity that [`on_model_ready`] built a collider for, so a re-fired
/// `WorldInstanceReady` rebuilds it instead of stacking a second one.
#[derive(Component, Clone, Copy, Debug)]
pub struct StaticMeshColliderPart;

pub struct StaticMeshColliderPlugin;

impl Plugin for StaticMeshColliderPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_model_ready);
    }
}

/// Builds the colliders once the tagged model's scene has spawned. Reacts to
/// `WorldInstanceReady` rather than polling `Assets<Mesh>`: by then every descendant `Mesh3d`
/// handle resolves.
///
/// Idempotent: the event can fire again for the same model (every instance of a model shares one
/// asset, and a modification of it respawns all instances), so previously built colliders are
/// removed first.
#[allow(clippy::too_many_arguments)]
pub fn on_model_ready(
    trigger: On<WorldInstanceReady>,
    tagged: Query<(), With<StaticMeshCollider>>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    transforms: Query<(), With<Transform>>,
    meshes_on: Query<&Mesh3d>,
    mesh_assets: Res<Assets<Mesh>>,
    built: Query<(), With<StaticMeshColliderPart>>,
    mut commands: Commands,
) {
    let model_root = trigger.entity;
    if !tagged.contains(model_root) {
        return;
    }
    let mut ancestor = model_root;
    loop {
        if !transforms.contains(ancestor) {
            warn!(
                ?ancestor,
                "scene entity has no Transform; avian will not scale the colliders below it"
            );
            break;
        }
        match parents.get(ancestor) {
            Ok(child_of) => ancestor = child_of.parent(),
            Err(_) => break,
        }
    }
    for descendant in children.iter_descendants(model_root) {
        if built.contains(descendant) {
            commands
                .entity(descendant)
                .remove::<(Collider, RigidBody, StaticMeshColliderPart)>();
        }
        let Ok(mesh_handle) = meshes_on.get(descendant) else {
            continue;
        };
        let Some(collider) = mesh_assets
            .get(&mesh_handle.0)
            .and_then(Collider::trimesh_from_mesh)
        else {
            continue;
        };
        commands
            .entity(descendant)
            .insert((collider, RigidBody::Static, StaticMeshColliderPart));
    }
}
