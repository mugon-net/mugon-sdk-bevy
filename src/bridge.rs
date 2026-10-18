//! Shared state between the Mugon JS callbacks and Bevy's per-frame systems.
//!
//! `Bridge` is registered exactly once, at [`crate::MugonPlugin::build`] time, via
//! [`Bridge::install`]. It reads the `MugonContext` that `Mugon.start` published as
//! `window.mugon` (role and ids are known synchronously — there is no handshake to wait for),
//! subscribes to its events once, and owns the three [`Closure`]s for the rest of the
//! process's lifetime, forwarding everything they receive into a plain `Rc<RefCell<_>>` queue
//! that ordinary Bevy systems drain every frame.
//!
//! `Bridge` is not `Send`/`Sync` (it holds `Rc`/`Closure`, which wrap JS values), so it must
//! be installed with [`bevy_app::App::insert_non_send`] and read with `NonSend`/`NonSendMut`,
//! not `Res`/`ResMut`. This is standard practice for wasm-bindgen state in Bevy;
//! `wasm32-unknown-unknown` browser builds are effectively single-threaded anyway.
//!
//! The queues are exposed raw rather than drained into Bevy events, since a consumer built on
//! top (e.g. Lightyear) wants "pop whatever's next" access, not a fixed event schema.

use crate::bindings::{self, Channel, MugonContext};
use crate::settings::{self, SettingValue};
use bytes::Bytes;
use js_sys::{Reflect, Uint8Array};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkMode {
    Server,
    Client,
}

#[derive(Default)]
pub(crate) struct BridgeState {
    /// Peer ids that connected, oldest first. On a server it starts out with the peers that
    /// were already connected when the game was initialised.
    pub(crate) connects: VecDeque<String>,
    /// Peer ids that disconnected, oldest first.
    pub(crate) disconnects: VecDeque<String>,
    /// `(from_peer_id, payload)` pairs received, oldest first.
    pub(crate) recv: VecDeque<(String, Bytes)>,
}

/// Owns the live JS-side registration for the whole process. See module docs.
pub struct Bridge {
    pub(crate) state: Rc<RefCell<BridgeState>>,
    context: MugonContext,
    network_mode: NetworkMode,
    own_id: String,
    server_id: String,
    settings: BTreeMap<String, SettingValue>,
    // Kept alive for as long as `Bridge` exists (i.e. for the app's lifetime) so JS can keep
    // calling into them. Never `.forget()`-leaked: dropping `Bridge` (which normally only
    // happens when the whole App/World goes away) drops these too.
    _on_connect: Closure<dyn FnMut(JsValue)>,
    _on_disconnect: Closure<dyn FnMut(JsValue)>,
    _on_message: Closure<dyn FnMut(JsValue)>,
}

fn get_string_field(obj: &JsValue, key: &str) -> Option<String> {
    Reflect::get(obj, &JsValue::from_str(key)).ok()?.as_string()
}

fn get_bytes_field(obj: &JsValue, key: &str) -> Option<Bytes> {
    let value = Reflect::get(obj, &JsValue::from_str(key)).ok()?;
    let array: Uint8Array = value.dyn_into().ok()?;
    Some(Bytes::from(array.to_vec()))
}

impl Bridge {
    /// Reads the published context and subscribes to its events. Must be called exactly once,
    /// from inside the callback of `Mugon.start`.
    ///
    /// # Panics
    ///
    /// If `window.mugon` is missing, or its role is neither `"server"` nor `"client"` (a
    /// protocol mismatch, not something to silently ignore).
    pub(crate) fn install() -> Self {
        let context = bindings::context();
        let network_mode = match context.role().as_str() {
            "server" => NetworkMode::Server,
            "client" => NetworkMode::Client,
            other => panic!("Mugon context has an unknown role: {other:?}"),
        };

        let state = Rc::new(RefCell::new(BridgeState::default()));
        // A client's only peer is its server, which its link is spawned for anyway; only a
        // server needs to hear about the peers that connected before it started.
        if network_mode == NetworkMode::Server {
            let mut state = state.borrow_mut();
            for peer in context.initial_peers().iter() {
                if let Some(peer_id) = peer.as_string() {
                    state.connects.push_back(peer_id);
                }
            }
        }

        let on_connect = {
            let state = state.clone();
            Closure::wrap(Box::new(move |peer_id: JsValue| {
                if let Some(peer_id) = peer_id.as_string() {
                    state.borrow_mut().connects.push_back(peer_id);
                }
            }) as Box<dyn FnMut(JsValue)>)
        };

        let on_disconnect = {
            let state = state.clone();
            Closure::wrap(Box::new(move |peer_id: JsValue| {
                if let Some(peer_id) = peer_id.as_string() {
                    state.borrow_mut().disconnects.push_back(peer_id);
                }
            }) as Box<dyn FnMut(JsValue)>)
        };

        let on_message = {
            let state = state.clone();
            Closure::wrap(Box::new(move |message: JsValue| {
                let from = get_string_field(&message, "from");
                let data = get_bytes_field(&message, "data");
                if let (Some(from), Some(data)) = (from, data) {
                    state.borrow_mut().recv.push_back((from, data));
                }
            }) as Box<dyn FnMut(JsValue)>)
        };

        context.on("connect", on_connect.as_ref().unchecked_ref());
        context.on("disconnect", on_disconnect.as_ref().unchecked_ref());
        context.on("message", on_message.as_ref().unchecked_ref());

        Self {
            state,
            network_mode,
            own_id: context.own_id(),
            server_id: context.server_id(),
            settings: settings::read_settings(&context.settings()),
            context,
            _on_connect: on_connect,
            _on_disconnect: on_disconnect,
            _on_message: on_message,
        }
    }

    /// Whether this process is the server or a client of the session.
    pub fn network_mode(&self) -> NetworkMode {
        self.network_mode
    }

    /// This peer's own id.
    pub fn own_id(&self) -> &str {
        &self.own_id
    }

    /// The server peer's id; equal to [`own_id`](Self::own_id) on the server itself.
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    /// Every setting the lobby chose a value for, in key order.
    pub fn settings(&self) -> impl Iterator<Item = (&str, &SettingValue)> {
        self.settings
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }

    /// The value of the setting `key`, whatever its type.
    pub fn setting(&self, key: &str) -> Option<&SettingValue> {
        self.settings.get(key)
    }

    /// A `boolean` setting; `None` if absent or declared with another type.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.setting(key)? {
            SettingValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// A `number` setting; `None` if absent or declared with another type.
    pub fn get_number(&self, key: &str) -> Option<f64> {
        match self.setting(key)? {
            SettingValue::Number(value) => Some(*value),
            _ => None,
        }
    }

    /// A `text` setting or a single-choice `select`; `None` if absent or declared with another
    /// type.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        match self.setting(key)? {
            SettingValue::Text(value) => Some(value),
            _ => None,
        }
    }

    /// A multi-choice `select`. A single-choice one reads as a one-element list, as in the
    /// JavaScript SDK; `None` if absent or declared with another type.
    pub fn get_strings(&self, key: &str) -> Option<Vec<&str>> {
        match self.setting(key)? {
            SettingValue::List(values) => Some(values.iter().map(String::as_str).collect()),
            SettingValue::Text(value) => Some(vec![value.as_str()]),
            _ => None,
        }
    }

    /// Deserializes all settings into `T`, whose field names (or `#[serde(rename = "...")]`s)
    /// are the setting keys. Needs the `serde` feature.
    #[cfg(feature = "serde")]
    pub fn settings_as<'de, T: serde::Deserialize<'de>>(&self) -> Result<T, crate::SettingsError> {
        settings::from_settings(&self.settings)
    }

    /// Pops the next queued peer id that connected, if any.
    pub fn pop_connect(&self) -> Option<String> {
        self.state.borrow_mut().connects.pop_front()
    }

    /// Pops the next queued peer id that disconnected, if any.
    pub fn pop_disconnect(&self) -> Option<String> {
        self.state.borrow_mut().disconnects.pop_front()
    }

    /// Pops the next queued `(from_peer_id, payload)`, if any.
    pub fn pop_recv(&self) -> Option<(String, Bytes)> {
        self.state.borrow_mut().recv.pop_front()
    }

    /// Sends `data` to `peer_id` on `channel`. Copies `data` into a fresh `Uint8Array` first —
    /// `ctx.send` transfers the buffer it's given, so it must never be a view into
    /// something else still owns (e.g. a transport's pooled packet buffers).
    pub fn send(&self, peer_id: &str, data: &[u8], channel: Channel) {
        let array = Uint8Array::new_with_length(data.len() as u32);
        array.copy_from(data);
        self.context.send(peer_id, &array, channel as u8);
    }

    /// `ctx.drop(peer_id)`.
    pub fn disconnect(&self, peer_id: &str) {
        self.context.drop_peer(peer_id);
    }

    /// `ctx.toggleFullscreen()`.
    pub fn toggle_fullscreen(&self) {
        self.context.toggle_fullscreen();
    }

    /// `ctx.showInvite()`.
    pub fn show_invite(&self) {
        self.context.show_invite();
    }

    /// `ctx.endGame()`: ends the game and sends everyone back to the lobby right away. Host
    /// only; on a client the call is dropped with a console warning.
    pub fn end_game(&self) {
        self.context.end_game();
    }

    /// `ctx.nextGame(settings)`: asks the host to start the next round. The host confirms
    /// first; settings the game version rejects end the game instead.
    ///
    /// `settings` replaces the lobby's values as a whole, so start from
    /// [`cloned_settings`](Self::cloned_settings), change what the next round needs and pass it
    /// back. A setting left out falls back to its default. Host only; on a client the call is
    /// dropped with a console warning.
    pub fn next_game(&self, settings: &BTreeMap<String, SettingValue>) {
        self.context.next_game(&settings::write_settings(settings));
    }

    /// An owned copy of [`settings`](Self::settings), to edit and hand to
    /// [`next_game`](Self::next_game).
    pub fn cloned_settings(&self) -> BTreeMap<String, SettingValue> {
        self.settings.clone()
    }

    /// Reports loading progress (`fraction` in `0.0..=1.0`) to the platform's loading overlay.
    pub fn set_progress(&self, fraction: f32, label: Option<&str>) {
        self.context
            .progress()
            .set(f64::from(fraction), label.map(str::to_owned));
    }

    /// Tells the platform the game is playable, which closes its loading overlay. The overlay
    /// otherwise only closes when the `Mugon.start` callback finishes, which — for a wasm
    /// game whose shell keeps that callback pending — never happens by itself.
    pub fn loaded(&self) {
        self.context.progress().done();
    }
}
