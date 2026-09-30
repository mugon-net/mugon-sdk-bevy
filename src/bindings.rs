//! `wasm-bindgen` externs matching the `MugonContext` of `mugon-sdk-js/src/gameframe.ts`.
//!
//! `Mugon.start` publishes the context as `window.mugon` before it runs the game's callback (so
//! before the wasm is started from it). Everything else in this crate is built on top of this
//! narrow surface.

use js_sys::Reflect;
use wasm_bindgen::prelude::*;

/// Mirrors `protocol.ts`'s `Channel` enum. Values must match the JS side exactly —
/// they are shared wire constants, not just a Rust-side convenience.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    ReliableOrdered = 0,
    ReliableUnordered = 1,
    UnreliableOrdered = 2,
    UnreliableUnordered = 3,
}

#[wasm_bindgen]
extern "C" {
    /// The `MugonContext` object.
    pub(crate) type MugonContext;

    /// `"server"` or `"client"`.
    #[wasm_bindgen(method, getter)]
    pub(crate) fn role(this: &MugonContext) -> String;

    #[wasm_bindgen(method, getter, js_name = ownId)]
    pub(crate) fn own_id(this: &MugonContext) -> String;

    /// The server peer's id; equal to `ownId` on the server itself.
    #[wasm_bindgen(method, getter, js_name = serverId)]
    pub(crate) fn server_id(this: &MugonContext) -> String;

    /// Snapshot of the peers connected when the game was initialised.
    #[wasm_bindgen(method, getter)]
    pub(crate) fn peers(this: &MugonContext) -> js_sys::Array;

    /// `ctx.send(peerId, data, channel)`.
    ///
    /// The JS side transfers (detaches) `data`'s backing `ArrayBuffer`. Callers must pass a
    /// `Uint8Array` that owns its buffer outright (e.g. a fresh copy), never a view into a
    /// buffer something else still expects to read from afterwards.
    #[wasm_bindgen(method)]
    pub(crate) fn send(this: &MugonContext, peer_id: &str, data: &js_sys::Uint8Array, channel: u8);

    /// `ctx.drop(peerId)`.
    #[wasm_bindgen(method, js_name = drop)]
    pub(crate) fn drop_peer(this: &MugonContext, peer_id: &str);

    /// `ctx.on(event, handler)`; returns the unsubscribe function. Events that arrived while
    /// no handler was registered are delivered to the first one.
    #[wasm_bindgen(method)]
    pub(crate) fn on(
        this: &MugonContext,
        event: &str,
        handler: &js_sys::Function,
    ) -> js_sys::Function;

    /// `ctx.showInvite()`.
    #[wasm_bindgen(method, js_name = showInvite)]
    pub(crate) fn show_invite(this: &MugonContext);

    /// `ctx.toggleFullscreen()`. Fullscreens the gameframe's own document, not the parent
    /// frame — a click's transient activation doesn't cross frames via postMessage.
    #[wasm_bindgen(method, js_name = toggleFullscreen)]
    pub(crate) fn toggle_fullscreen(this: &MugonContext);

    #[wasm_bindgen(method, getter)]
    pub(crate) fn progress(this: &MugonContext) -> MugonProgress;

    /// `ctx.progress`.
    pub(crate) type MugonProgress;

    /// `ctx.progress.set(fraction, label?)`.
    #[wasm_bindgen(method)]
    pub(crate) fn set(this: &MugonProgress, fraction: f64, label: Option<String>);

    /// `ctx.progress.done()`.
    #[wasm_bindgen(method)]
    pub(crate) fn done(this: &MugonProgress);
}

/// The context `Mugon.start` published as `window.mugon`.
///
/// # Panics
///
/// If it isn't there: the wasm must be started from inside `Mugon.start`'s callback.
pub(crate) fn context() -> MugonContext {
    Reflect::get(&js_sys::global(), &JsValue::from_str("mugon"))
        .ok()
        .filter(|value| !value.is_undefined() && !value.is_null())
        .expect(
            "window.mugon is missing: start the wasm from inside the callback of \
             `Mugon.start` (@mugon/sdk)",
        )
        .unchecked_into()
}
