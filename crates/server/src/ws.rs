//! WebSocket transport (MG3.1 + MG3.3): one socket, two logical channels.
//!
//! `GET /ws` upgrades to a WebSocket that bridges a client to the per-canvas
//! actor. Two *logical* channels are multiplexed over the single socket; the
//! channel is a property of the message type, not a separate stream:
//!
//! - **reliable_ordered** — `hello`/`welcome`, `ops`/`ack`/`rejected`, `patch`.
//!   These ride the actor's ordered op pipeline: every `ops` apply is sequenced
//!   and acked, and applied patches fan out to peers in seq order.
//! - **ephemeral_besteffort** — `presence`. Lossy by design: a lagging receiver
//!   drops the oldest frames rather than back-pressuring, and nothing is
//!   persisted. Presence rides the registry's per-canvas presence broadcast.
//!
//! Scope: each `ops` entry is now an MG-4 [`OpEnvelope`] — a whole
//! [`RenderScenePatch`] wrapped with its `opId` (`clientId` + `localSeq`),
//! `baseRevision`, and a client `ts`. The actor dedups by `opId` (idempotent
//! re-apply), and `ack`/`rejected` echo the `opIds` they resolved. The granular
//! per-property `propDelta` wire shape ([`shape_scene_core::wire`]'s `WireOp`) is
//! a later refinement and is intentionally not used here. This envelope is defined
//! in this crate (reusing scene-core's [`RenderScenePatch`]/[`Scene`]) so the two
//! op shapes never collide on serde's internally-tagged `type` discriminator.
//!
//! Identity is `userId`-only with no auth (C13): the op's `opId.clientId` doubles
//! as the authoring user id passed to the actor.

use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use shape_scene_core::{Bounds, CanvasId, RenderScenePatch, Scene};
use tokio::sync::mpsc;

use crate::canvas_actor::{patch_touches_region, ApplyResult};
use crate::registry::CanvasRegistry;
use crate::sync::{OpEnvelope, OpId};

// ---------------------------------------------------------------------------
// MG-3 WS envelope (client <-> server). Internally tagged on `type`, camelCase.
// ---------------------------------------------------------------------------

/// Upstream (client -> server) messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WsClientMessage {
    /// Open the session on a canvas. The first message a client sends; the
    /// server replies with [`WsServerMessage::Welcome`].
    ///
    /// `userId` (MG-6.3) is the connection's attributed author. It is optional and
    /// permissive (single-user / no-auth, C13): when present it is the self-skip
    /// identity for op echo (MG-6.1) and the value a real authz hook would gate on;
    /// when absent the connection falls back to a per-connection id.
    #[serde(rename_all = "camelCase")]
    Hello {
        canvas_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<Region>,
        #[serde(default)]
        last_ack_seq: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_id: Option<String>,
    },
    /// A batch of op ENVELOPES to apply, in order (MG4.2). Each envelope carries
    /// its own `opId` (`clientId` + `localSeq`) for idempotent dedup, the
    /// `baseRevision` it was authored against, a client `ts`, and the whole render
    /// `patch`. A duplicate `opId` is a no-op that re-acks the original seq.
    #[serde(rename_all = "camelCase")]
    Ops { ops: Vec<OpEnvelope> },
    /// (Re)subscribe to a region of the canvas. MG-3 stores the region for MG-9
    /// windowing and otherwise broadcasts the whole canvas.
    #[serde(rename_all = "camelCase")]
    Subscribe { canvas_id: String, region: Region },
    /// A best-effort presence frame fanned out to the canvas's other clients.
    #[serde(rename_all = "camelCase")]
    Presence {
        canvas_id: String,
        payload: serde_json::Value,
    },
    /// Resume after a disconnect from `lastAckSeq`. MG-3 replies with a fresh
    /// `welcome` snapshot; gap replay from the journal is MG-4.
    #[serde(rename_all = "camelCase")]
    Resume {
        canvas_id: String,
        last_ack_seq: i64,
    },
}

/// Downstream (server -> client) messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WsServerMessage {
    /// Handshake reply: the current scene snapshot plus the server seq/revision.
    #[serde(rename_all = "camelCase")]
    Welcome {
        scene: Scene,
        seq: i64,
        revision: i64,
    },
    /// One applied op: the `opIds` it acked (one per op) plus the server
    /// seq/revision after it. A duplicate op re-acks its original seq/revision.
    #[serde(rename_all = "camelCase")]
    Ack {
        op_ids: Vec<OpId>,
        seq: i64,
        revision: i64,
    },
    /// One op rejected by scene-core; nothing was applied for it. `opIds` echoes
    /// the rejected op's id(s) so the client can fail the matching outbox entry.
    #[serde(rename_all = "camelCase")]
    Rejected {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        op_ids: Vec<OpId>,
        errors: Vec<String>,
    },
    /// A peer's applied patch fanned out in seq order.
    #[serde(rename_all = "camelCase")]
    Patch { ops: Vec<RenderScenePatch>, seq: i64 },
    /// A peer's presence frame (ephemeral/best-effort).
    #[serde(rename_all = "camelCase")]
    Presence { payload: serde_json::Value },
    /// A transport- or protocol-level error (bad frame, premature ops, …).
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

/// A windowed subscription into a canvas. `bbox == None` is the whole canvas.
/// Kept structurally identical to [`shape_scene_core::wire::Region`] so the MG-3
/// and MG-4 wire shapes agree; stored for MG-9 windowing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub canvas_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<shape_scene_core::Bounds>,
}

// ---------------------------------------------------------------------------
// Handler.
// ---------------------------------------------------------------------------

/// `GET /ws`: upgrade to a WebSocket bridged to the per-canvas actor.
pub async fn ws_handler(
    State(canvases): State<CanvasRegistry>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| handle_socket(socket, canvases))
}

/// Per-connection driver. Owns the socket from `hello` to close.
async fn handle_socket(socket: WebSocket, canvases: CanvasRegistry) {
    let (mut sink, mut stream) = socket.split();

    // 1) Await the opening `hello` (anything else first is a protocol error).
    let hello = loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<WsClientMessage>(&text) {
                Ok(WsClientMessage::Hello {
                    canvas_id,
                    region,
                    last_ack_seq,
                    user_id,
                }) => break (canvas_id, region, last_ack_seq, user_id),
                Ok(_) => {
                    let _ = send(&mut sink, &error("expected hello as the first message")).await;
                    return;
                }
                Err(e) => {
                    let _ = send(&mut sink, &error(&format!("invalid hello: {e}"))).await;
                    return;
                }
            },
            // Ignore pre-hello control frames; close/error/EOF ends the session.
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Binary(_))) => {
                let _ = send(&mut sink, &error("binary frames are not supported")).await;
                return;
            }
            _ => return,
        }
    };
    let (canvas_str, hello_region, _last_ack_seq, hello_user_id) = hello;
    let canvas_id = CanvasId::from(canvas_str.as_str());

    // The connection's current windowed region (MG9.5). Shared with the patch
    // fan-out task so a `subscribe` from the reader loop re-aims the live filter.
    // `None` bbox is the whole canvas. Seeded from `hello.region`.
    let region: Arc<Mutex<Option<Bounds>>> =
        Arc::new(Mutex::new(hello_region.and_then(|r| r.bbox)));

    // MG-6.1 self-skip: the author ids this connection writes with. The patch
    // fan-out task skips a broadcast whose `author` is in this set, because the
    // originator already applied the op optimistically; peers still receive it.
    // Seeded with the hello `userId` (the attributed author, MG-6.3) and grown
    // with each op's `clientId` so a client that authors under several ids never
    // echoes any of its own.
    let self_authors: Arc<Mutex<std::collections::HashSet<String>>> = {
        let mut set = std::collections::HashSet::new();
        if let Some(uid) = &hello_user_id {
            set.insert(uid.clone());
        }
        Arc::new(Mutex::new(set))
    };

    // 2) Acquire the lease + spawn the actor (loads durable state on first use)
    //    and subscribe to BOTH fan-outs before sending welcome, so no patch
    //    between snapshot and subscribe is missed. A lease held by another owner
    //    (MG8.2a) or a draining registry (MG8.3) ends the session with an error.
    let handle = match canvases.get_or_spawn(&canvas_id).await {
        Ok(handle) => handle,
        Err(e) => {
            let _ = send(&mut sink, &error(&format!("cannot open canvas: {e}"))).await;
            return;
        }
    };
    let mut patch_rx = handle.subscribe();
    let mut presence_rx = canvases.presence_subscribe(&canvas_id);

    // The welcome snapshot is region-filtered (MG9.5): a windowed connection only
    // receives the objects in its region. `revision` is read from the FULL scene
    // so the windowed client still reconciles against the true canvas revision.
    let bbox = *region.lock().expect("region mutex poisoned");
    let scene = handle.get_scene_region(bbox).await;
    let revision = scene.scene_version;
    // The actor exposes its seq only via broadcasts/apply results; the welcome
    // seq mirrors the scene revision for MG-3 (one apply == one revision bump).
    let welcome = WsServerMessage::Welcome {
        scene,
        seq: revision,
        revision,
    };
    if send(&mut sink, &welcome).await.is_err() {
        return;
    }

    // 3) A single writer task owns the sink; reader + fan-out tasks funnel
    //    outbound frames through `out_tx` so the sink is never shared.
    let (out_tx, mut out_rx) = mpsc::channel::<WsServerMessage>(256);
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if send(&mut sink, &msg).await.is_err() {
                break;
            }
        }
    });

    // 4) Op fan-out: actor broadcast -> client `patch`.
    //    MG-6.1: the actor's `PatchBroadcast` now carries the op's `author`. This
    //    task SKIPS a broadcast whose author is one of this connection's own ids
    //    (the originator already applied it optimistically); peer connections
    //    still receive it. MG-9.5 region filtering applies on top.
    let patch_out = out_tx.clone();
    let patch_region = Arc::clone(&region);
    let patch_authors = Arc::clone(&self_authors);
    let patch_task = tokio::spawn(async move {
        loop {
            match patch_rx.recv().await {
                Ok(b) => {
                    // MG-6.1 self-skip: do not echo an op back to its originator.
                    if let Some(author) = &b.author {
                        if patch_authors
                            .lock()
                            .expect("self-authors mutex poisoned")
                            .contains(author)
                        {
                            continue;
                        }
                    }
                    // MG9.5 fan-out filter: deliver only if this op touches the
                    // connection's current region. The broadcast carries the
                    // post-apply scene, so the filter can locate moved/created
                    // objects; deletes/structural ops are sent through.
                    let bbox = *patch_region.lock().expect("region mutex poisoned");
                    if !patch_touches_region(&b.scene, &b.patch, bbox) {
                        continue;
                    }
                    let msg = WsServerMessage::Patch {
                        ops: vec![b.patch],
                        seq: b.seq,
                    };
                    if patch_out.send(msg).await.is_err() {
                        break;
                    }
                }
                // Lagged: the actor channel is ordered+reliable, so on lag we
                // skip the dropped frames and keep forwarding the live tail.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 5) Presence fan-out: registry presence broadcast -> client `presence`.
    //    MG-6.2: a presence frame from one connection is fanned out to the OTHER
    //    connections of the same canvas. This task skips a frame whose `from` is
    //    one of this connection's own author ids, so a connection never receives
    //    its own cursor (best-effort, never persisted, latest-wins per user is
    //    the client's concern).
    let presence_out = out_tx.clone();
    let presence_authors = Arc::clone(&self_authors);
    let presence_task = tokio::spawn(async move {
        loop {
            match presence_rx.recv().await {
                Ok(frame) => {
                    if presence_authors
                        .lock()
                        .expect("self-authors mutex poisoned")
                        .contains(&frame.from)
                    {
                        continue; // do not echo our own presence back.
                    }
                    let msg = WsServerMessage::Presence {
                        payload: frame.payload,
                    };
                    // Best-effort: if the client's outbound buffer is full, drop
                    // this presence frame rather than block ops behind it.
                    let _ = presence_out.try_send(msg);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 6) Reader loop: client frames -> actor / presence fan-out.
    // userId-only identity (C13): the connection's attributed author is the hello
    // `userId` when supplied, else a stable per-CONNECTION id (not per-canvas, so
    // two connections on one canvas never share a self-skip identity). Ops carry
    // their own `clientId`, which takes precedence when present.
    let fallback_user_id = hello_user_id.unwrap_or_else(connection_author_id);
    while let Some(frame) = stream.next().await {
        let text = match frame {
            Ok(Message::Text(t)) => t,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Binary(_)) => {
                let _ = out_tx.send(error("binary frames are not supported")).await;
                continue;
            }
        };

        let msg: WsClientMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                let _ = out_tx.send(error(&format!("invalid message: {e}"))).await;
                continue;
            }
        };

        match msg {
            // A second hello on an open session is ignored (idempotent open).
            WsClientMessage::Hello { .. } => {}
            WsClientMessage::Ops { ops } => {
                for envelope in ops {
                    // The authoring identity is the op's clientId (userId-only,
                    // C13); fall back to the connection id when it is empty.
                    let author = if envelope.op_id.client_id.is_empty() {
                        fallback_user_id.clone()
                    } else {
                        envelope.op_id.client_id.clone()
                    };

                    // MG-6.3 write gate (single-line seam, permissive C13).
                    // TODO(auth): real authz decides accept/reject for `author`
                    // on this canvas here; today every attributed write passes.
                    if !authorize_write(&author, &canvas_id) {
                        let _ = out_tx
                            .send(WsServerMessage::Rejected {
                                op_ids: vec![envelope.op_id.clone()],
                                errors: vec!["write not authorized".to_string()],
                            })
                            .await;
                        continue;
                    }

                    // MG-6.1 self-skip: remember this author so the fan-out task
                    // never echoes the op back to this connection.
                    self_authors
                        .lock()
                        .expect("self-authors mutex poisoned")
                        .insert(author.clone());

                    let op_id = envelope.op_id.clone();
                    match handle.apply_envelope(envelope, &author).await {
                        ApplyResult::Applied { seq, revision } => {
                            if out_tx
                                .send(WsServerMessage::Ack {
                                    op_ids: vec![op_id],
                                    seq,
                                    revision,
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        ApplyResult::Rejected { errors } => {
                            if out_tx
                                .send(WsServerMessage::Rejected {
                                    op_ids: vec![op_id],
                                    errors,
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            }
            WsClientMessage::Subscribe { region: new_region, .. } => {
                // MG9.5: re-aim the window. Update the shared region (the fan-out
                // task reads it on the next broadcast), then send a fresh
                // region-filtered snapshot so the client reconciles — loading
                // objects that entered the new window and evicting those that
                // exited. `welcome` is reused as the snapshot frame.
                let bbox = new_region.bbox;
                *region.lock().expect("region mutex poisoned") = bbox;
                let scene = handle.get_scene_region(bbox).await;
                let revision = scene.scene_version;
                if out_tx
                    .send(WsServerMessage::Welcome {
                        scene,
                        seq: revision,
                        revision,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            WsClientMessage::Presence {
                canvas_id: presence_canvas,
                payload,
            } => {
                // MG-6.2 best-effort fan-out to the canvas's OTHER clients. The
                // frame is tagged with this connection's author so the fan-out
                // task skips echoing it back to us (we never render our own
                // cursor); peers still receive it.
                self_authors
                    .lock()
                    .expect("self-authors mutex poisoned")
                    .insert(fallback_user_id.clone());
                let target = CanvasId::from(presence_canvas.as_str());
                canvases.presence_publish(&target, &fallback_user_id, payload);
            }
            WsClientMessage::Resume { .. } => {
                // MG-3: reply with a fresh snapshot; journal gap-replay is MG-4.
                // The snapshot honours the connection's current region (MG9.5).
                let bbox = *region.lock().expect("region mutex poisoned");
                let scene = handle.get_scene_region(bbox).await;
                let revision = scene.scene_version;
                if out_tx
                    .send(WsServerMessage::Welcome {
                        scene,
                        seq: revision,
                        revision,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }

    // Reader ended: drop the outbound sender so the writer drains and exits, and
    // abort the fan-out tasks (their receivers are tied to this connection).
    drop(out_tx);
    patch_task.abort();
    presence_task.abort();
    let _ = writer.await;
}

/// A stable, process-unique fallback author id for a connection that did not
/// advertise a `userId` (MG-6.1 self-skip needs a per-connection identity, not a
/// shared per-canvas one). Deterministic within a process; no rng.
fn connection_author_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("ws-conn-{}-{n}", std::process::id())
}

/// MG-6.3 write gate hook: decide whether `author` may write to `canvas_id`.
///
/// Permissive by design (single-user / no-auth, C13): every attributed write is
/// allowed today. This is the seam where real authz attaches.
// TODO(auth): replace the permissive default with a real policy lookup.
fn authorize_write(_author: &str, _canvas_id: &CanvasId) -> bool {
    true
}

fn error(message: &str) -> WsServerMessage {
    WsServerMessage::Error {
        message: message.to_string(),
    }
}

/// Serialize `msg` to JSON text and push it onto `sink`.
async fn send<S>(sink: &mut S, msg: &WsServerMessage) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = serde_json::to_string(msg).expect("server message serializes");
    sink.send(Message::Text(text)).await.map_err(|_| ())
}
