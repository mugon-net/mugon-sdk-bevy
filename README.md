# mugon-sdk-bevy

Bevy integration for games running on the [mugon.net](https://mugon.net) platform.

The platform embeds your game in a sandboxed iframe and hands it a context once the network is
up: whether this process is the **server** (host) or a **client**, its own id, the server's id,
and the peers already connected. This crate exposes that context to a Bevy app and lets the game
report loading progress to the platform's loading overlay. It is deliberately independent of any
networking layer; for [Lightyear](https://github.com/cBournhonesque/lightyear) see
`lightyear_mugon`.

## Requirements

- The wasm must be started from inside the callback of `Mugon.start` from
  [`@mugon/sdk`](https://www.npmjs.com/package/@mugon/sdk), which publishes the context as
  `window.mugon` before running the callback. The `mugon init` bevy template sets this up.
- Target `wasm32-unknown-unknown`, built with `wasm-bindgen` (`--target web`). The `wasm-bindgen`
  CLI version has to match the `wasm-bindgen` crate version in your `Cargo.lock`.

## Usage

```rust
use bevy::prelude::*;
use mugon_sdk_bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, MugonPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(bridge: NonSend<Bridge>) {
    info!("running as {:?} with id {}", bridge.network_mode(), bridge.own_id());
    // Close the platform's loading overlay: the game is playable.
    bridge.loaded();
}
```

`Bridge` is a `NonSend` resource (it wraps JS values). It offers `network_mode()`, `own_id()`,
`server_id()`, queues to drain (`pop_connect`, `pop_disconnect`, `pop_recv`), `send`,
`disconnect`, `toggle_fullscreen`, `show_invite`, `set_progress` and `loaded`.

### Loading progress

With the `loading` feature, `MugonLoadingPlugin` reports asset loading to the overlay:

```rust
fn setup(asset_server: Res<AssetServer>, mut loading: ResMut<MugonLoading>) {
    loading.track(asset_server.load::<Image>("sprite.png"));
}
```

Track everything the game needs before it is playable; the overlay closes once all tracked
assets have loaded (or failed). Games with nothing to load call `Bridge::loaded()` directly.

### Touch controls

The `touch-controls` feature adds joystick/button input components and a fullscreen toggle.

## License

Licensed under either of MIT or Apache-2.0, at your option.
