//! WebSocket transport: `GET /ws` bridges a client to the per-canvas actor. Two
//! logical channels multiplex over the single socket (channel = message type):
//!
//! - reliable_ordered — `hello`/`welcome`, `ops`/`ack`/`rejected`, `patch`, and
//!   `feature` frames, riding the actor's ordered op pipeline.
//! - ephemeral_besteffort — `presence`. Lossy by design and never persisted.
//!
//! Each `ops` entry is a [`WireOp`](shape_scene_core::wire::WireOp) whose
//! `propDelta` carries the [`ObjectOp`] JSON plus the `opId` (`clientId` +
//! `localSeq`) and `baseRevision`; the actor dedups by `opId`. Identity is
//! `userId`-only with no auth: `opId.clientId` doubles as the authoring user id.

use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use shape_scene_core::object::{FeatureRequest, FeatureResponse, ObjectOp, ObjectScene};
use shape_scene_core::wire::{OpId as WireOpId, WireOp};
use shape_scene_core::{Bounds, CanvasId};
use shape_storage_core::RegionWindow;
use tokio::sync::mpsc;

use crate::canvas_actor::ApplyResult;
use crate::object_store::object_region_key;
use crate::registry::CanvasRegistry;
use crate::sync::{OpEnvelope, OpId};

/// Upstream (client -> server) messages. Internally tagged on `type`, camelCase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WsClientMessage {
    /// The first message a client sends; the server replies with `Welcome`.
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
    #[serde(rename_all = "camelCase")]
    Ops { ops: Vec<WireOp> },
    /// Lowered to ops on the single op-apply path.
    #[serde(rename_all = "camelCase")]
    Feature { request: FeatureRequest },
    #[serde(rename_all = "camelCase")]
    Subscribe { canvas_id: String, region: Region },
    #[serde(rename_all = "camelCase")]
    Presence {
        canvas_id: String,
        payload: serde_json::Value,
    },
    /// Replies with a fresh `welcome` snapshot.
    #[serde(rename_all = "camelCase")]
    Resume {
        canvas_id: String,
        last_ack_seq: i64,
    },
}

/// Downstream (server -> client) messages. Internally tagged on `type`, camelCase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WsServerMessage {
    #[serde(rename_all = "camelCase")]
    Welcome {
        scene: ObjectScene,
        seq: i64,
        revision: i64,
    },
    /// A duplicate op re-acks its original seq/revision.
    #[serde(rename_all = "camelCase")]
    Ack {
        op_ids: Vec<OpId>,
        seq: i64,
        revision: i64,
    },
    #[serde(rename_all = "camelCase")]
    Rejected {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        op_ids: Vec<OpId>,
        errors: Vec<String>,
    },
    /// A peer's applied op fanned out in seq order.
    #[serde(rename_all = "camelCase")]
    Patch { ops: Vec<WireOp>, seq: i64 },
    #[serde(rename_all = "camelCase")]
    Feature { response: FeatureResponse },
    #[serde(rename_all = "camelCase")]
    Presence { payload: serde_json::Value },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

/// A windowed subscription. `bbox == None` is the whole canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub canvas_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Bounds>,
}

fn bounds_to_window(bounds: Bounds) -> RegionWindow {
    RegionWindow {
        min_x: bounds.x,
        min_y: bounds.y,
        max_x: bounds.x + bounds.width,
        max_y: bounds.y + bounds.height,
    }
}

/// `propDelta` carries the internally-tagged [`ObjectOp`] JSON; an error string
/// lets the caller reject a malformed delta.
fn wire_op_to_envelope(wire: &WireOp) -> Result<OpEnvelope, String> {
    let op: ObjectOp = serde_json::from_value(wire.prop_delta.clone())
        .map_err(|e| format!("invalid op delta: {e}"))?;
    Ok(OpEnvelope {
        op_id: OpId {
            client_id: wire.op_id.client_id.clone(),
            local_seq: wire.op_id.local_seq,
        },
        base_revision: wire.base_revision,
        ts: wire.ts.clone(),
        op,
    })
}

/// `objectId`/`kind` are descriptive; `propDelta` carries the op JSON the peer
/// re-applies.
fn op_to_wire(op: &ObjectOp, seq: i64, author: &str) -> WireOp {
    let object_id = op.target_ids().first().cloned().unwrap_or_default();
    WireOp {
        op_id: WireOpId {
            client_id: author.to_string(),
            local_seq: seq,
        },
        object_id,
        kind: op.kind().to_string(),
        prop_delta: serde_json::to_value(op).expect("op serializes"),
        base_revision: seq,
        actor: author.to_string(),
        ts: String::new(),
    }
}

/// Whether an applied op reaches a connection windowed to `bbox`. Conservative: a
/// whole-canvas subscriber (`None`) gets everything; a windowed subscriber gets
/// it unless every target is locatable in the post-apply scene AND outside the
/// window.
fn op_touches_region(scene: &ObjectScene, op: &ObjectOp, bbox: Option<Bounds>) -> bool {
    let Some(bbox) = bbox else { return true };
    let window = bounds_to_window(bbox);
    let targets = op.target_ids();
    if targets.is_empty() {
        return true;
    }
    // The canvas id only shapes the RegionKey; bounds are world-space.
    let canvas = CanvasId::from("w");
    let mut located_any = false;
    for id in &targets {
        if let Some(object) = scene.get(id) {
            if let Some(key) = object_region_key(&canvas, object) {
                located_any = true;
                let overlaps = key.min_x <= window.max_x
                    && window.min_x <= key.max_x
                    && key.min_y <= window.max_y
                    && window.min_y <= key.max_y;
                if overlaps {
                    return true;
                }
            }
        }
    }
    // No locatable target => structural/unlocalizable op, send through; all located
    // targets outside the window => drop.
    !located_any
}

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

    // Await the opening `hello`; anything else first is a protocol error.
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

    // Current windowed region, shared with the patch fan-out task so a `subscribe`
    // re-aims the live filter. `None` bbox is the whole canvas.
    let region: Arc<Mutex<Option<Bounds>>> =
        Arc::new(Mutex::new(hello_region.and_then(|r| r.bbox)));

    // The author ids this connection writes with: the patch fan-out skips a
    // broadcast whose `author` is in this set (the originator already applied it).
    let self_authors: Arc<Mutex<std::collections::HashSet<String>>> = {
        let mut set = std::collections::HashSet::new();
        if let Some(uid) = &hello_user_id {
            set.insert(uid.clone());
        }
        Arc::new(Mutex::new(set))
    };

    // Subscribe to BOTH fan-outs before sending welcome so no patch between
    // snapshot and subscribe is missed.
    let handle = match canvases.get_or_spawn(&canvas_id).await {
        Ok(handle) => handle,
        Err(e) => {
            let _ = send(&mut sink, &error(&format!("cannot open canvas: {e}"))).await;
            return;
        }
    };
    let mut patch_rx = handle.subscribe();
    let mut presence_rx = canvases.presence_subscribe(&canvas_id);

    // The welcome snapshot is region-filtered to the connection's window.
    let window = region
        .lock()
        .expect("region mutex poisoned")
        .map(bounds_to_window);
    let scene = handle.get_scene_region(window).await;
    let revision = scene.scene_version;
    let welcome = WsServerMessage::Welcome {
        scene,
        seq: revision,
        revision,
    };
    if send(&mut sink, &welcome).await.is_err() {
        return;
    }

    // A single writer task owns the sink; reader + fan-out tasks funnel outbound
    // frames through `out_tx` so the sink is never shared.
    let (out_tx, mut out_rx) = mpsc::channel::<WsServerMessage>(256);
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if send(&mut sink, &msg).await.is_err() {
                break;
            }
        }
    });

    // Op fan-out: actor broadcast -> client `patch`, skipping this connection's
    // own author ids.
    let patch_out = out_tx.clone();
    let patch_region = Arc::clone(&region);
    let patch_authors = Arc::clone(&self_authors);
    let patch_task = tokio::spawn(async move {
        loop {
            match patch_rx.recv().await {
                Ok(b) => {
                    if let Some(author) = &b.author {
                        if patch_authors
                            .lock()
                            .expect("self-authors mutex poisoned")
                            .contains(author)
                        {
                            continue;
                        }
                    }
                    let bbox = *patch_region.lock().expect("region mutex poisoned");
                    if !op_touches_region(&b.scene, &b.op, bbox) {
                        continue;
                    }
                    let author = b.author.as_deref().unwrap_or("");
                    let msg = WsServerMessage::Patch {
                        ops: vec![op_to_wire(&b.op, b.seq, author)],
                        seq: b.seq,
                    };
                    if patch_out.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Presence fan-out: registry presence broadcast -> client `presence`.
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
                        continue;
                    }
                    let msg = WsServerMessage::Presence {
                        payload: frame.payload,
                    };
                    let _ = presence_out.try_send(msg);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Reader loop: client frames -> actor / presence fan-out.
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
            WsClientMessage::Hello { .. } => {}
            WsClientMessage::Ops { ops } => {
                for wire in ops {
                    let envelope = match wire_op_to_envelope(&wire) {
                        Ok(e) => e,
                        Err(message) => {
                            let _ = out_tx
                                .send(WsServerMessage::Rejected {
                                    op_ids: vec![OpId {
                                        client_id: wire.op_id.client_id.clone(),
                                        local_seq: wire.op_id.local_seq,
                                    }],
                                    errors: vec![message],
                                })
                                .await;
                            continue;
                        }
                    };

                    // Authoring identity is the op's clientId; fall back to the
                    // connection id when empty.
                    let author = if envelope.op_id.client_id.is_empty() {
                        fallback_user_id.clone()
                    } else {
                        envelope.op_id.client_id.clone()
                    };

                    if !authorize_write(&author, &canvas_id) {
                        let _ = out_tx
                            .send(WsServerMessage::Rejected {
                                op_ids: vec![envelope.op_id.clone()],
                                errors: vec!["write not authorized".to_string()],
                            })
                            .await;
                        continue;
                    }

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
            WsClientMessage::Feature { request } => {
                self_authors
                    .lock()
                    .expect("self-authors mutex poisoned")
                    .insert(fallback_user_id.clone());
                let response = handle.feature(request, &fallback_user_id).await;
                if out_tx
                    .send(WsServerMessage::Feature { response })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            WsClientMessage::Subscribe { region: new_region, .. } => {
                let bbox = new_region.bbox;
                *region.lock().expect("region mutex poisoned") = bbox;
                let scene = handle.get_scene_region(bbox.map(bounds_to_window)).await;
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
                self_authors
                    .lock()
                    .expect("self-authors mutex poisoned")
                    .insert(fallback_user_id.clone());
                let target = CanvasId::from(presence_canvas.as_str());
                canvases.presence_publish(&target, &fallback_user_id, payload);
            }
            WsClientMessage::Resume { .. } => {
                let bbox = *region.lock().expect("region mutex poisoned");
                let scene = handle.get_scene_region(bbox.map(bounds_to_window)).await;
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

    // Drop the outbound sender so the writer drains and exits; abort the fan-out
    // tasks tied to this connection.
    drop(out_tx);
    patch_task.abort();
    presence_task.abort();
    let _ = writer.await;
}

/// Fallback author id when a connection advertises no `userId`. Deterministic
/// within a process; no rng.
fn connection_author_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("ws-conn-{}-{n}", std::process::id())
}

/// Write-gate hook, permissive by design (no-auth). TODO(auth): real authz here.
fn authorize_write(_author: &str, _canvas_id: &CanvasId) -> bool {
    true
}

fn error(message: &str) -> WsServerMessage {
    WsServerMessage::Error {
        message: message.to_string(),
    }
}

async fn send<S>(sink: &mut S, msg: &WsServerMessage) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = serde_json::to_string(msg).expect("server message serializes");
    sink.send(Message::Text(text)).await.map_err(|_| ())
}

