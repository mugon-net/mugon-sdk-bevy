//! Bevy integration for games running on the [Mugon](https://mugon.net) platform.
//!
//! Wraps [`lightyear_mugon::MugonPlugin`] (the wasm transport, see that crate for the
//! full design) together with Lightyear's [`ClientPlugins`]/[`ServerPlugins`], and
//! surfaces peer connect/disconnect as plain Bevy events so game code doesn't need to
//! know anything about Lightyear's connection-lifecycle components to spawn/despawn
//! per-player entities.
//!
//! Deliberately POC-scoped (see `games/mario-kart-poc/PLAN.md`): this is only what
//! that game needs, not a general-purpose SDK. `ClientPlugins`/`ServerPlugins` are
//! always both installed, matching `lightyear_mugon`'s own decision to resolve
//! client/server role at runtime rather than compile time (a single wasm build can be
//! told by the platform to be either).

use bevy_app::{App, Plugin};
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::*;
use lightyear::prelude::*;
use lightyear_mugon::MugonPlugin as MugonTransportPlugin;

/// Fired when a peer's Lightyear connection entity gets [`Connected`].
///
/// On the server this fires once per connected client (the `LinkOf` child entity);
/// on the client it fires once, for its single connection to the server. `peer_id`
/// is Lightyear's own identity for the link (a [`PeerId::Raw`] wrapping the synthetic
/// `SocketAddr` `lightyear_mugon` assigns per platform peer id) — sufficient for
/// `ControlledBy`/`NetworkTarget::Single` and friends; the platform's own string peer
/// id isn't exposed outside `lightyear_mugon` today (its `peer_addr` module is
/// private), which is fine since gameplay code only needs Lightyear-level identity.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerConnected {
    pub entity: Entity,
    pub peer_id: PeerId,
}

/// Fired when a peer's Lightyear connection entity gets [`Disconnected`]. See
/// [`PeerConnected`] for what `peer_id` is.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerDisconnected {
    pub entity: Entity,
    pub peer_id: PeerId,
}

pub struct MugonBevyPlugin;

impl Plugin for MugonBevyPlugin {
    fn build(&self, app: &mut App) {
        // Facade plugin groups first, `MugonTransportPlugin` second -- confirmed
        // live (Milestone 1 checkpoint) that the other order panics.
        // `server::ServerPlugins`'s `PluginGroupBuilder` adds
        // `lightyear_link::server::ServerLinkPlugin` unconditionally, with no
        // `is_plugin_added` guard of its own (that check only happens once, when a
        // plugin *group* is finished and applied to the `App`). `lightyear_mugon`'s
        // `MugonServerPlugin` also wants `ServerLinkPlugin`, but guards its own add
        // with `is_plugin_added`. Whichever side runs first "wins" the unguarded
        // slot; the second side's guard (if it has one) then correctly no-ops. So
        // `ServerPlugins`/`ClientPlugins` must go first, `MugonTransportPlugin`
        // second -- the reverse order panics at startup with "plugin was already
        // added in application". Do not additionally enable the `lightyear` facade
        // crate's `raw_connection`/`netcode` cargo features in this crate's
        // Cargo.toml -- either one would make `ClientPlugins`/`ServerPlugins` try to
        // add `RawConnectionPlugin` a second time (that one `lightyear_mugon` adds
        // unconditionally, with no guard at all), hitting the same failure mode.
        app.add_plugins((client::ClientPlugins::default(), server::ServerPlugins::default()));
        app.add_plugins(MugonTransportPlugin);

        // `lightyear_prediction::RollbackPlugin` (always installed by `ClientPlugins`'s default
        // `prediction` feature) schedules `reset_input_rollback_tracker` unconditionally in
        // `PreUpdate`, and that system takes a hard `ResMut<LastConfirmedInput>` -- but
        // `LastConfirmedInput` is only initialized when a `PredictionManager` resource is
        // inserted (`PredictionPlugin`'s `On<Insert, PredictionManager>` observer), which is
        // opt-in and no Mugon game had opted into yet. Without it, that system panics with
        // "Resource does not exist" on every tick, even with zero predicted entities. The actual
        // prediction/rollback pipeline still stays dormant per-topology (`should_run` requires
        // `NetworkTopology::Client`/`P2P`) and per-component (`PredictionTarget`), so inserting
        // this unconditionally on both roles is safe -- it only prevents this crash.
        app.insert_resource(PredictionManager::default());

        app.add_message::<PeerConnected>();
        app.add_message::<PeerDisconnected>();
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

/// Opts every connected link into replication and emits [`PeerConnected`].
///
/// Neither `ReplicationSender` (server-side, per connected client link) nor
/// `ReplicationReceiver` (the client's own connection entity) are inserted
/// automatically by Lightyear's replicon backend -- a link that lacks them stays
/// connected (messages still flow) but never replicates entities. Every Mugon game
/// wants replication on for every peer, so this is handled here once rather than in
/// each game.
fn on_connected(
    trigger: On<Add, Connected>,
    remote_ids: Query<&RemoteId>,
    is_client_of: Query<(), With<server::ClientOf>>,
    is_client: Query<(), With<Client>>,
    mut events: MessageWriter<PeerConnected>,
    mut commands: Commands,
) {
    if is_client_of.get(trigger.entity).is_ok() {
        commands.entity(trigger.entity).insert(ReplicationSender);
    }
    if is_client.get(trigger.entity).is_ok() {
        commands.entity(trigger.entity).insert(ReplicationReceiver);
    }
    if let Ok(remote_id) = remote_ids.get(trigger.entity) {
        events.write(PeerConnected {
            entity: trigger.entity,
            peer_id: remote_id.0,
        });
    }
}

fn on_disconnected(
    trigger: On<Add, Disconnected>,
    remote_ids: Query<&RemoteId>,
    mut events: MessageWriter<PeerDisconnected>,
) {
    if let Ok(remote_id) = remote_ids.get(trigger.entity) {
        events.write(PeerDisconnected {
            entity: trigger.entity,
            peer_id: remote_id.0,
        });
    }
}
