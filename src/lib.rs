//! Bevy integration for games running on the [Mugon](https://mugon.net) platform, independent
//! of any particular networking layer.
//!
//! [`MugonPlugin`] installs the JS bridge to the `MugonContext` that `mugon-sdk-js`'s
//! `Mugon.start` publishes as `window.mugon` (see [`Bridge`]); the wasm must be started from
//! inside that callback. The role and ids are known as soon as the plugin is built. It
//! deliberately doesn't turn the bridge's queues into Bevy events itself: a networking layer
//! built on top (e.g. `lightyear_mugon`'s `MugonLightyearPlugin`) decides how to consume them.
//!
//! The lobby's chosen setting values are read through [`Bridge`] (`get_bool`, `get_number`,
//! `get_string`, `get_strings`, `settings`), or with the optional `serde` feature deserialized into a
//! struct via [`Bridge::settings_as`].
//!
//! The optional `loading` feature adds [`MugonLoadingPlugin`], which reports asset loading to
//! the platform's loading overlay.
//!
//! The optional `touch-controls` feature adds joystick/button input components and a
//! fullscreen toggle — see [`touch_controls`] for how spawning them works.

mod bindings;
mod bridge;
#[cfg(feature = "loading")]
mod loading;
mod settings;
#[cfg(feature = "touch-controls")]
pub mod touch_controls;

use bevy_app::{App, Plugin};

pub use bindings::Channel;
pub use bridge::{Bridge, NetworkMode};
#[cfg(feature = "loading")]
pub use loading::{MugonLoading, MugonLoadingPlugin};
pub use settings::SettingValue;
#[cfg(feature = "serde")]
pub use settings::SettingsError;
#[cfg(feature = "touch-controls")]
pub use touch_controls::{
    IsTouchDevice, MugonFullscreenButton, MugonFullscreenButtonPlugin, MugonJoystick,
    MugonJoystickKnob, MugonTouchButton, MugonTouchControlsPlugin,
};

pub mod prelude {
    pub use crate::{Bridge, Channel, MugonPlugin, NetworkMode, SettingValue};
    #[cfg(feature = "touch-controls")]
    pub use crate::{
        IsTouchDevice, MugonFullscreenButton, MugonFullscreenButtonPlugin, MugonJoystick,
        MugonJoystickKnob, MugonTouchButton, MugonTouchControlsPlugin,
    };
    #[cfg(feature = "loading")]
    pub use crate::{MugonLoading, MugonLoadingPlugin};
}

/// Installs the Mugon JS bridge as a `NonSend` resource. See module docs.
pub struct MugonPlugin;

impl Plugin for MugonPlugin {
    fn build(&self, app: &mut App) {
        app.insert_non_send(Bridge::install());
    }
}
