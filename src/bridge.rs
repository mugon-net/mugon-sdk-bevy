//! Shared state between the Mugon JS callbacks and Bevy's per-frame systems.
//!
//! `Bridge` is registered exactly once, at [`crate::MugonPlugin::build`] time, via
//! [`Bridge::install`]. It reads the `MugonContext` that `Mugon.start` published as
//! `window.mugon` (role and players are known synchronously — there is no handshake to wait for),
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

/// The character a player picked on the platform. `code` is a mugon-characters code (what
/// `mugon-characters-bevy` renders from); the URLs serve the baked glTF (glb) and mugshot (webp)
/// for games that don't use that library. A snapshot taken when the player connected, and the
/// default character's until the player's own bake is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerCharacter {
    pub code: String,
    pub model_url: String,
    pub mugshot_url: String,
}

/// A participant in the game. The name is the player's display name, supplied by the
/// platform; it is not unique, so identify players by `id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub character: Option<PlayerCharacter>,
}

/// The part this player takes in the game's network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkRole {
    Server,
    Client,
}

#[derive(Default)]
pub(crate) struct BridgeState {
    /// Players that connected, oldest first. On a server it starts out with the players that
    /// were already connected when the game was initialised.
    pub(crate) connects: VecDeque<Player>,
    /// Ids of the players that disconnected, oldest first.
    pub(crate) disconnects: VecDeque<String>,
    /// `(from_player_id, payload)` pairs received, oldest first.
    pub(crate) recv: VecDeque<(String, Bytes)>,
}

/// Owns the live JS-side registration for the whole process. See module docs.
pub struct Bridge {
    pub(crate) state: Rc<RefCell<BridgeState>>,
    context: MugonContext,
    network_role: NetworkRole,
    self_player: Player,
    server_player: Player,
    settings: BTreeMap<String, SettingValue>,
    carry_over: Option<String>,
    character_editor: Option<JsValue>,
    /// Codes the host page asked the character editor to load, oldest first.
    character_loads: Rc<RefCell<VecDeque<String>>>,
    _on_character_load: Option<Closure<dyn FnMut(JsValue)>>,
    // Kept alive for as long as `Bridge` exists (i.e. for the app's lifetime) so JS can keep
    // calling into them. Never `.forget()`-leaked: dropping `Bridge` (which normally only
    // happens when the whole App/World goes away) drops these too.
    _on_connect: Closure<dyn FnMut(JsValue)>,
    _on_disconnect: Closure<dyn FnMut(JsValue)>,
    _on_message: Closure<dyn FnMut(JsValue)>,
}

fn read_player(value: &JsValue) -> Option<Player> {
    Some(Player {
        id: get_string_field(value, "id")?,
        name: get_string_field(value, "name")?,
        character: Reflect::get(value, &JsValue::from_str("character"))
            .ok()
            .and_then(|character| read_character(&character)),
    })
}

fn read_character(value: &JsValue) -> Option<PlayerCharacter> {
    Some(PlayerCharacter {
        code: get_string_field(value, "code")?,
        model_url: get_string_field(value, "modelUrl")?,
        mugshot_url: get_string_field(value, "mugshotUrl")?,
    })
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
        let network_role = match context.role().as_str() {
            "server" => NetworkRole::Server,
            "client" => NetworkRole::Client,
            other => panic!("Mugon context has an unknown role: {other:?}"),
        };

        let state = Rc::new(RefCell::new(BridgeState::default()));
        // A client's only player is its server, which its link is spawned for anyway; only a
        // server needs to hear about the players that connected before it started.
        if network_role == NetworkRole::Server {
            let mut state = state.borrow_mut();
            for player in context.initial_players().iter() {
                if let Some(player) = read_player(&player) {
                    state.connects.push_back(player);
                }
            }
        }

        let on_connect = {
            let state = state.clone();
            Closure::wrap(Box::new(move |player: JsValue| {
                if let Some(player) = read_player(&player) {
                    state.borrow_mut().connects.push_back(player);
                }
            }) as Box<dyn FnMut(JsValue)>)
        };

        let on_disconnect = {
            let state = state.clone();
            Closure::wrap(Box::new(move |player_id: JsValue| {
                if let Some(player_id) = player_id.as_string() {
                    state.borrow_mut().disconnects.push_back(player_id);
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

        let character_editor = Some(context.character_editor()).filter(|editor| !editor.is_undefined());
        let character_loads = Rc::new(RefCell::new(VecDeque::new()));
        let on_character_load = character_editor.as_ref().and_then(|editor| {
            let queue = character_loads.clone();
            let closure = Closure::wrap(Box::new(move |code: JsValue| {
                if let Some(code) = code.as_string() {
                    queue.borrow_mut().push_back(code);
                }
            }) as Box<dyn FnMut(JsValue)>);
            let on_load = Reflect::get(editor, &JsValue::from_str("onLoad")).ok()?;
            on_load.dyn_ref::<js_sys::Function>()?.call1(editor, closure.as_ref().unchecked_ref()).ok()?;
            Some(closure)
        });

        context.on("playerConnected", on_connect.as_ref().unchecked_ref());
        context.on("playerDisconnected", on_disconnect.as_ref().unchecked_ref());
        context.on("message", on_message.as_ref().unchecked_ref());

        Self {
            state,
            network_role,
            self_player: read_player(&context.self_player())
                .expect("Mugon context has no valid `self` player"),
            server_player: read_player(&context.server_player())
                .expect("Mugon context has no valid `server` player"),
            settings: settings::read_settings(&context.settings()),
            carry_over: context.carry_over(),
            character_editor,
            character_loads,
            _on_character_load: on_character_load,
            context,
            _on_connect: on_connect,
            _on_disconnect: on_disconnect,
            _on_message: on_message,
        }
    }

    /// Whether this process is the server or a client of the session.
    pub fn network_role(&self) -> NetworkRole {
        self.network_role
    }

    /// This player.
    pub fn self_player(&self) -> &Player {
        &self.self_player
    }

    /// The server player; equal to [`self_player`](Self::self_player) on the server itself.
    pub fn server_player(&self) -> &Player {
        &self.server_player
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

    /// Pops the next queued player that connected, if any.
    pub fn pop_connect(&self) -> Option<Player> {
        self.state.borrow_mut().connects.pop_front()
    }

    /// Pops the next queued id of a player that disconnected, if any.
    pub fn pop_disconnect(&self) -> Option<String> {
        self.state.borrow_mut().disconnects.pop_front()
    }

    /// Pops the next queued `(from_player_id, payload)`, if any.
    pub fn pop_recv(&self) -> Option<(String, Bytes)> {
        self.state.borrow_mut().recv.pop_front()
    }

    /// Sends `data` to `player_id` on `channel`. Copies `data` into a fresh `Uint8Array` first —
    /// `ctx.send` transfers the buffer it's given, so it must never be a view into
    /// something else still owns (e.g. a transport's pooled packet buffers).
    pub fn send(&self, player_id: &str, data: &[u8], channel: Channel) {
        let array = Uint8Array::new_with_length(data.len() as u32);
        array.copy_from(data);
        self.context.send(player_id, &array, channel as u8);
    }

    /// `ctx.disconnectPlayer(player_id)`.
    pub fn disconnect_player(&self, player_id: &str) {
        self.context.disconnect_player(player_id);
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

    /// `ctx.requestNextRound(settings, { carryOver })`: asks the host to start the next round. The
    /// host confirms first; settings the game version rejects end the game instead.
    ///
    /// `carry_over` is any string up to 1 MiB that [`carry_over`](Self::carry_over) returns in the
    /// next round; a longer one drops the whole call with a console warning. It is not secret.
    ///
    /// `settings` replaces the lobby's values as a whole, so start from
    /// [`cloned_settings`](Self::cloned_settings), change what the next round needs and pass it
    /// back. A setting left out falls back to its default. Host only; on a client the call is
    /// dropped with a console warning.
    pub fn next_round(&self, settings: &BTreeMap<String, SettingValue>, carry_over: Option<&str>) {
        let options = js_sys::Object::new();
        if let Some(carry_over) = carry_over {
            let _ = Reflect::set(
                &options,
                &JsValue::from_str("carryOver"),
                &JsValue::from_str(carry_over),
            );
        }
        self.context
            .request_next_round(&settings::write_settings(settings), &options);
    }

    /// The player's current character code, when the platform runs this game as the character
    /// editor of the profile page; `None` in every other game.
    pub fn character_editor_code(&self) -> Option<String> {
        get_string_field(self.character_editor.as_ref()?, "code")
    }

    /// The next code the host page asked the character editor to load (a code the player pasted
    /// there), oldest first. Answer with [`report_character_code`](Self::report_character_code) once
    /// switched, or [`character_load_failed`](Self::character_load_failed). Always `None` outside
    /// the character editor.
    pub fn pop_character_load(&self) -> Option<String> {
        self.character_loads.borrow_mut().pop_front()
    }

    /// Tells the host page the character editor could not apply a code from
    /// [`pop_character_load`](Self::pop_character_load). Only the character editor has this.
    pub fn character_load_failed(&self, reason: &str) {
        let Some(editor) = &self.character_editor else { return };
        let Ok(failed) = Reflect::get(editor, &JsValue::from_str("loadFailed")) else { return };
        if let Some(failed) = failed.dyn_ref::<js_sys::Function>() {
            let _ = failed.call1(editor, &JsValue::from_str(reason));
        }
    }

    /// Tells the platform the code the player's character has now (call it on every change; the
    /// platform saves the latest one when the player confirms). Only the character editor has
    /// this; elsewhere the call does nothing.
    pub fn report_character_code(&self, code: &str) {
        let Some(editor) = &self.character_editor else { return };
        let Ok(edited) = Reflect::get(editor, &JsValue::from_str("edited")) else { return };
        if let Some(edited) = edited.dyn_ref::<js_sys::Function>() {
            let _ = edited.call1(editor, &JsValue::from_str(code));
        }
    }

    /// What the previous round handed over through [`next_round`](Self::next_round). Only the
    /// host gets it, and only in the round right after; `None` otherwise (also after a reload).
    pub fn carry_over(&self) -> Option<&str> {
        self.carry_over.as_deref()
    }

    /// An owned copy of [`settings`](Self::settings), to edit and hand to
    /// [`next_round`](Self::next_round).
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
