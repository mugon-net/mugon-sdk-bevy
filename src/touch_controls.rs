//! Optional on-screen touch controls (feature `touch-controls`): virtual joysticks and
//! touch/mouse buttons, plus a fullscreen toggle. This module only runs the input logic;
//! spawning is up to the consumer — add [`MugonJoystick`]/[`MugonTouchButton`] to whatever
//! entity (with whatever `Node` size/position/styling) you want to act as one, and any number
//! of them can exist at once.
//!
//! Hit-testing bypasses `bevy_ui::Interaction` and hit-tests each control's own
//! `ComputedNode`/`UiGlobalTransform` directly, latching onto whichever pointer acquired it —
//! `ui_focus_system` collapses every active pointer into one "cursor position" per camera and
//! resets *every* `Pressed` node when *any* pointer releases, both of which break multi-touch
//! (e.g. holding a button while dragging a joystick with another finger).

use crate::bindings;
use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_input::prelude::{ButtonInput, MouseButton, Touches};
use bevy_math::Vec2;
use bevy_ui::prelude::*;
use bevy_window::Window;

/// Whether this peer is on a touch-capable device. Set once at startup by
/// [`MugonTouchControlsPlugin`]; whether to spawn controls at all is up to the consumer.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsTouchDevice(pub bool);

/// `navigator.maxTouchPoints > 0` — not a user-agent sniff, which is brittle and easy to
/// spoof/miss.
#[cfg(target_arch = "wasm32")]
fn detect_touch_device() -> bool {
    web_sys::window().is_some_and(|window| window.navigator().max_touch_points() > 0)
}

#[cfg(not(target_arch = "wasm32"))]
fn detect_touch_device() -> bool {
    false
}

/// Latches onto one pointer (touch id, or the desktop-testing mouse) so a second pointer
/// elsewhere can't hijack an in-progress drag/press.
#[derive(Default)]
struct PointerLatch {
    touch_id: Option<u64>,
    mouse: bool,
}

/// A virtual joystick. Add to any UI entity — its own `Node` sets the joystick's position and
/// size (a circle inscribed in that `Node`, radius = half the shorter side). `value`'s axes are
/// each in `[-1, 1]` (already flipped so "up" is positive); read it to drive gameplay.
///
/// A child entity carrying [`MugonJoystickKnob`] is optional; if present, its `Node.left`/`.top`
/// are driven each frame to visually track the drag (it must use
/// `PositionType::Absolute`, sized and placed however you like).
#[derive(Component, Default)]
pub struct MugonJoystick {
    pub value: Vec2,
    drag_origin: Option<Vec2>,
    latch: PointerLatch,
}

/// Marks a child of a [`MugonJoystick`] entity as its visual knob. See that type's docs.
#[derive(Component)]
pub struct MugonJoystickKnob;

/// A touch/mouse button. Add to any UI entity alongside your own marker component (to tell
/// buttons apart) and read `pressed`. Multiple can exist at once, each tracking its own pointer
/// independently.
#[derive(Component, Default)]
pub struct MugonTouchButton {
    pub pressed: bool,
    latch: PointerLatch,
}

/// Runs the [`MugonJoystick`]/[`MugonTouchButton`] input systems and detects [`IsTouchDevice`].
/// Does not spawn anything — see the module docs.
pub struct MugonTouchControlsPlugin;

impl Plugin for MugonTouchControlsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(IsTouchDevice(detect_touch_device()));
        app.add_systems(Update, (update_joysticks, update_touch_buttons));
    }
}

fn window_cursor_position(windows: &Query<&Window>) -> Option<Vec2> {
    windows.single().ok().and_then(Window::cursor_position)
}

/// Physical-pixel `ComputedNode::size` halved and converted to logical pixels, taking the
/// shorter side so a non-square `Node` still yields a sane inscribed-circle radius.
fn node_radius(node: &ComputedNode) -> f32 {
    node.size.min_element() * node.inverse_scale_factor() * 0.5
}

/// Drives every [`MugonJoystick`] from whichever pointer *started* the current drag on its own
/// `Node` — falls back to the mouse cursor for desktop testing without a touchscreen. Tracks the
/// drag as an offset from wherever the pointer started (a "floating" joystick, not anchored to
/// the base's absolute screen position).
///
/// `ComputedNode::contains_point` hit-testing works in **physical** pixels (per
/// `ComputedNode::inverse_scale_factor`'s own doc comment); `touch.position()`/
/// `window.cursor_position()` are logical, so the hit-test point is scaled before the call
/// (matching `ui_focus_system`'s own `* window.scale_factor()` for touch) — the *offset* math
/// stays entirely in logical space, so only the hit-test argument is ever scaled. (Skips
/// `ui_focus_system`'s viewport-origin subtract since this assumes one full-window UI camera.)
fn update_joysticks(
    mut joysticks: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        &mut MugonJoystick,
        Option<&Children>,
    )>,
    mut knobs: Query<(&ComputedNode, &mut Node), With<MugonJoystickKnob>>,
    touches: Res<Touches>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
) {
    for (node, transform, mut joystick, children) in &mut joysticks {
        let radius = node_radius(node);
        let to_physical = |position: Vec2| position / node.inverse_scale_factor();
        let hits_base = |position: Vec2| node.contains_point(*transform, to_physical(position));

        let pointer_position = if let Some(touch) = joystick
            .latch
            .touch_id
            .and_then(|id| touches.get_pressed(id))
        {
            Some(touch.position())
        } else if joystick.latch.mouse && mouse.pressed(MouseButton::Left) {
            window_cursor_position(&windows)
        } else if let Some(touch) = touches
            .iter_just_pressed()
            .find(|touch| hits_base(touch.position()))
        {
            joystick.drag_origin = Some(touch.position());
            joystick.latch.touch_id = Some(touch.id());
            Some(touch.position())
        } else if mouse.just_pressed(MouseButton::Left)
            && window_cursor_position(&windows).is_some_and(hits_base)
        {
            let position = window_cursor_position(&windows);
            joystick.drag_origin = position;
            joystick.latch.mouse = true;
            position
        } else {
            None
        };

        let offset = match pointer_position {
            Some(position) => {
                let origin = *joystick.drag_origin.get_or_insert(position);
                (position - origin).clamp_length_max(radius)
            }
            None => {
                joystick.drag_origin = None;
                joystick.latch = PointerLatch::default();
                Vec2::ZERO
            }
        };
        // Screen space is y-down; "up" (positive y) must read as a positive joystick value.
        joystick.value = if radius > 0.0 {
            Vec2::new(offset.x / radius, -offset.y / radius)
        } else {
            Vec2::ZERO
        };

        let Some(children) = children else { continue };
        for &child in children {
            let Ok((knob_node, mut knob_style)) = knobs.get_mut(child) else {
                continue;
            };
            let knob_radius = node_radius(knob_node);
            knob_style.left = Val::Px(radius - knob_radius + offset.x);
            knob_style.top = Val::Px(radius - knob_radius + offset.y);
        }
    }
}

/// Whether a button is held, via the same direct hit-test/latch [`update_joysticks`] uses
/// instead of `bevy_ui`'s `Interaction`. Once a pointer latches on, it stays "pressed" for as
/// long as that pointer is down, regardless of whether it's still over the button — matching
/// `Interaction::Pressed`'s own sticky-through-drag behavior, which this replaces.
fn touch_button_pressed(
    hits: impl Fn(Vec2) -> bool,
    touches: &Touches,
    mouse: &ButtonInput<MouseButton>,
    cursor_position: Option<Vec2>,
    latch: &mut PointerLatch,
) -> bool {
    if let Some(id) = latch.touch_id {
        if touches.get_pressed(id).is_some() {
            return true;
        }
        latch.touch_id = None;
    }
    if latch.mouse {
        if mouse.pressed(MouseButton::Left) {
            return true;
        }
        latch.mouse = false;
    }
    if let Some(touch) = touches
        .iter_just_pressed()
        .find(|touch| hits(touch.position()))
    {
        latch.touch_id = Some(touch.id());
        return true;
    }
    if mouse.just_pressed(MouseButton::Left) && cursor_position.is_some_and(&hits) {
        latch.mouse = true;
        return true;
    }
    false
}

/// Drives every [`MugonTouchButton`], hit-testing each one's own `ComputedNode`/
/// `UiGlobalTransform` independently — see `touch_button_pressed`. Each button latches its own
/// pointer, so a joystick touch (or another button's touch) active in the same frame can never
/// suppress or misroute its press.
fn update_touch_buttons(
    mut buttons: Query<(&ComputedNode, &UiGlobalTransform, &mut MugonTouchButton)>,
    touches: Res<Touches>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
) {
    let cursor_position = window_cursor_position(&windows);
    for (node, transform, mut button) in &mut buttons {
        let to_physical = |position: Vec2| position / node.inverse_scale_factor();
        let hits = |position: Vec2| node.contains_point(*transform, to_physical(position));
        button.pressed =
            touch_button_pressed(hits, &touches, &mouse, cursor_position, &mut button.latch);
    }
}

/// Marks a UI entity as the always-visible fullscreen toggle. Add it to a `Button` entity of
/// whatever size/position/styling you like; pressing it calls `window.requestFullscreen()`.
#[derive(Component)]
pub struct MugonFullscreenButton;

/// Runs [`MugonFullscreenButton`]'s input handling. Does not spawn anything — see that type's
/// docs.
pub struct MugonFullscreenButtonPlugin;

impl Plugin for MugonFullscreenButtonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_fullscreen_button);
    }
}

fn handle_fullscreen_button(
    interactions: Query<&Interaction, (Changed<Interaction>, With<MugonFullscreenButton>)>,
) {
    if let Some(Interaction::Pressed) = interactions.iter().next() {
        bindings::context().toggle_fullscreen();
    }
}
