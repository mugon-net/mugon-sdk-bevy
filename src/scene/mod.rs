//! Helpers for levels exported by the Mugon editor (`mugon-editor export`).
//!
//! An exported scene file spawns the hierarchy: names, transforms, `WorldAssetRoot` models,
//! [`Marker`] volumes and the game's own components. What a game then needs is small and
//! generic, and lives here:
//!
//! - [`Marker`]/[`MarkerShape`]: the volume placeholders the exported scenes use.
//! - [`world_transform`]: an entity's world pose right after spawning, before Bevy's transform
//!   propagation has run (at `Startup`, `GlobalTransform` is still the identity).
//! - [`markers`]: every entity carrying a component, as a [`MarkerVolume`] with its world pose,
//!   surface point and a point-in-volume test.
//!
//! With the `avian` feature, [`collider`] builds static trimesh colliders for the models of
//! tagged scene entities.
//!
//! What a marker *means* (checkpoints, spawns, ...) is up to the game.

#[cfg(feature = "avian")]
pub mod collider;

use bevy_ecs::prelude::*;
use bevy_math::{Vec2, Vec3};
use bevy_transform::components::{GlobalTransform, Transform};

/// Shape of a [`Marker`] volume. The volume is the unit shape (box: 1x1x1, sphere and cylinder:
/// diameter 1, cylinder height 1 along Y) scaled by the entity's `Transform`.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerShape {
    #[default]
    Box,
    Sphere,
    Cylinder,
}

/// A volume placeholder placed in the editor. It has no rendering of its own.
#[derive(Component, Default, Clone, Debug)]
pub struct Marker {
    pub shape: MarkerShape,
}

/// The entity's pose in world space, composed by hand from its `Transform` and those of its
/// ancestors. An entity without a `Transform` counts as the identity.
///
/// Use this instead of `GlobalTransform` for scenes spawned this frame (e.g. in `Startup`).
pub fn world_transform(world: &World, entity: Entity) -> Transform {
    global_transform(world, entity).compute_transform()
}

fn global_transform(world: &World, entity: Entity) -> GlobalTransform {
    let local = world.get::<Transform>(entity).copied().unwrap_or_default();
    match world.get::<ChildOf>(entity) {
        Some(child_of) => global_transform(world, child_of.parent()).mul_transform(local),
        None => GlobalTransform::from(local),
    }
}

/// An entity carrying a component of interest, with where it is.
#[derive(Clone, Debug)]
pub struct MarkerVolume<C> {
    pub entity: Entity,
    pub component: C,
    /// `None` if the entity is not a [`Marker`] (a plain model with the component).
    pub shape: Option<MarkerShape>,
    /// World pose; for a marker the scale is the size of the volume.
    pub transform: Transform,
}

impl<C> MarkerVolume<C> {
    pub fn center(&self) -> Vec3 {
        self.transform.translation
    }

    /// The point on the surface the volume stands on: its centre dropped by half its height
    /// along its own up axis. The editor places a marker's centre half a height above the
    /// ground, so this is what to snap spawns and checkpoints to.
    pub fn base(&self) -> Vec3 {
        self.transform.translation - self.transform.rotation * Vec3::Y * (self.transform.scale.y * 0.5)
    }

    /// Whether `point` (world space) lies inside the volume. A volume without a shape is
    /// treated as a box.
    pub fn contains(&self, point: Vec3) -> bool {
        let local = self.transform.compute_affine().inverse().transform_point3(point);
        match self.shape.unwrap_or_default() {
            MarkerShape::Box => local.abs().max_element() <= 0.5,
            MarkerShape::Sphere => local.length() <= 0.5,
            MarkerShape::Cylinder => Vec2::new(local.x, local.z).length() <= 0.5 && local.y.abs() <= 0.5,
        }
    }
}

/// Every entity with component `C`, with its world pose (see [`world_transform`]).
pub fn markers<C: Component + Clone>(world: &mut World) -> Vec<MarkerVolume<C>> {
    let found: Vec<(Entity, C)> = world
        .query::<(Entity, &C)>()
        .iter(world)
        .map(|(entity, component)| (entity, component.clone()))
        .collect();
    found
        .into_iter()
        .map(|(entity, component)| MarkerVolume {
            entity,
            component,
            shape: world.get::<Marker>(entity).map(|marker| marker.shape),
            transform: world_transform(world, entity),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_math::Quat;

    #[derive(Component, Clone, Debug, PartialEq)]
    struct Spawn(u32);

    #[test]
    fn world_transform_composes_ancestors_without_propagation() {
        let mut world = World::new();
        let group = world.spawn_empty().id();
        let parent = world
            .spawn((Transform::from_xyz(10.0, 0.0, 0.0).with_scale(Vec3::splat(2.0)), ChildOf(group)))
            .id();
        let child = world.spawn((Transform::from_xyz(1.0, 0.0, 0.0), ChildOf(parent))).id();
        let pose = world_transform(&world, child);
        assert_eq!(pose.translation, Vec3::new(12.0, 0.0, 0.0));
        assert_eq!(pose.scale, Vec3::splat(2.0));
    }

    #[test]
    fn markers_report_pose_shape_and_base() {
        let mut world = World::new();
        let group = world.spawn(Transform::from_xyz(0.0, 5.0, 0.0)).id();
        world.spawn((
            Spawn(3),
            Marker { shape: MarkerShape::Box },
            Transform::from_xyz(0.0, 0.5, 0.0).with_scale(Vec3::new(2.0, 1.0, 3.0)),
            ChildOf(group),
        ));
        world.spawn(Spawn(4));
        let mut found = markers::<Spawn>(&mut world);
        found.sort_by_key(|volume| volume.component.0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].shape, Some(MarkerShape::Box));
        assert_eq!(found[0].center(), Vec3::new(0.0, 5.5, 0.0));
        assert_eq!(found[0].base(), Vec3::new(0.0, 5.0, 0.0));
        assert_eq!(found[1].shape, None);
    }

    #[test]
    fn contains_honours_shape_rotation_and_scale() {
        let volume = |shape, transform| MarkerVolume {
            entity: Entity::PLACEHOLDER,
            component: (),
            shape: Some(shape),
            transform,
        };
        let long_box = volume(
            MarkerShape::Box,
            Transform::from_rotation(Quat::from_rotation_y(core::f32::consts::FRAC_PI_2)).with_scale(Vec3::new(1.0, 1.0, 10.0)),
        );
        assert!(long_box.contains(Vec3::new(4.9, 0.0, 0.0)), "rotated long axis points along x");
        assert!(!long_box.contains(Vec3::new(0.0, 0.0, 4.9)));

        let ball = volume(MarkerShape::Sphere, Transform::from_scale(Vec3::splat(4.0)));
        assert!(ball.contains(Vec3::new(1.9, 0.0, 0.0)));
        assert!(!ball.contains(Vec3::new(1.5, 1.5, 0.0)));

        let pillar = volume(MarkerShape::Cylinder, Transform::from_scale(Vec3::new(2.0, 6.0, 2.0)));
        assert!(pillar.contains(Vec3::new(0.9, 2.9, 0.0)));
        assert!(!pillar.contains(Vec3::new(0.9, 3.1, 0.0)));
        assert!(!pillar.contains(Vec3::new(0.8, 0.0, 0.8)));
    }
}
