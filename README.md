# mugon-sdk-bevy

Bevy integration for games running on the [mugon.net](https://mugon.net) platform.

The platform embeds your game in a sandboxed iframe and hands it a context once the network is
up: whether this process is the **server** (host) or a **client**, its own id, the server's id,
and the peers already connected. This crate exposes that context to a Bevy app and lets the game
report loading progress to the platform's loading overlay. It is deliberately independent of any
networking layer; for [Lightyear](https://github.com/cBournhonesque/lightyear) see
`mugon-sdk-lightyear`.

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
`disconnect`, `toggle_fullscreen`, `show_invite`, `end_game`, `next_game`, `set_progress` and `loaded`.

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

### Editor scenes

The `scene` feature has helpers for levels exported by the Mugon editor: the `Marker`/`MarkerShape` volumes the exported scenes use,
`world_transform` (an entity's world pose right after spawning, before Bevy's transform propagation has run) and
`markers::<C>(world)` (every entity carrying component `C`, with its world pose, surface point and a point-in-volume test).
The `avian` feature adds `StaticMeshColliderPlugin` and the `StaticMeshCollider` tag, which build static trimesh colliders for the
models of tagged scene entities.

## License

[MIT](LICENSE-MIT)

## Links

- Repository and issues: <https://github.com/mugon-net/mugon-sdk-bevy>
- JavaScript SDK: <https://github.com/mugon-net/mugon-sdk-js>
- Lightyear transport: <https://github.com/mugon-net/mugon-sdk-lightyear>
- CLI: <https://github.com/mugon-net/mugon-cli>
- Platform: <https://mugon.net>
