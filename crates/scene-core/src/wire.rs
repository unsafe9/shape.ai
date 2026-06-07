//! WebSocket wire protocol serde types (MG0.2d — 전송/sync seam).
//!
//! These are the on-the-wire messages exchanged over the transport's two logical
//! channels. The shapes mirror the task-breakdown sync handshake:
//!   `hello{canvasId,region,lastAckSeq}` → `welcome{snapshot(region)|deltaSince}`
//!   → live; upstream `ops[{opId,objectId,kind,propDelta,baseRevision,actor,ts}]`
//!   → `ack{opIds,seq,revision}`; downstream `patch{ops,seq}`; presence is
//!   ephemeral; region change re-subscribes; reconnect = snapshot+reapply.
//!
//! Like the rest of scene-core this layer is platform-pure: clocks, ids, and
//! sequence numbers are carried as plain fields, never sourced ambiently here.
//!
//! FUTURE: WebTransport / gRPC stream transports can map onto these same logical
//! channels (O14).

use serde::{Deserialize, Serialize};

use crate::model::Bounds;
use crate::object::ObjectScene;

// ---------------------------------------------------------------------------
// Subscription region.
// ---------------------------------------------------------------------------

/// A windowed subscription into a canvas. `bbox == None` subscribes to the whole
/// canvas; a `Some(bbox)` constrains the working set to that world-space rect
/// (data-layer windowing for memory-bounded large canvases, PC10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub canvas_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Bounds>,
}

// ---------------------------------------------------------------------------
// Per-op idempotency key + the granular op envelope (MG4.2).
// ---------------------------------------------------------------------------

/// `(clientId, localSeq)` idempotency key assigned at the transport boundary.
/// The server dedups by this pair so a replayed outbox entry is a no-op.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpId {
    pub client_id: String,
    pub local_seq: i64,
}

/// A single granular operation as it travels over the wire: a per-property delta
/// against a base revision, attributed to a user, with a client clock stamp.
/// `prop_delta` stays an opaque JSON value so the wire schema does not couple to
/// the full op union (the canvas actor decodes it server-side).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireOp {
    pub op_id: OpId,
    pub object_id: String,
    pub kind: String,
    pub prop_delta: serde_json::Value,
    pub base_revision: i64,
    /// userId of the authoring actor.
    pub actor: String,
    pub ts: String,
}

// ---------------------------------------------------------------------------
// Client → Server messages.
// ---------------------------------------------------------------------------

/// Upstream messages. Internally tagged on `type`; the single-word variant names
/// stay lowercase under camelCase rename (`hello`, `subscribe`, …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMessage {
    #[serde(rename_all = "camelCase")]
    Hello {
        canvas_id: String,
        region: Region,
        last_ack_seq: i64,
    },
    #[serde(rename_all = "camelCase")]
    Subscribe { canvas_id: String, region: Region },
    #[serde(rename_all = "camelCase")]
    Ops { ops: Vec<WireOp> },
    #[serde(rename_all = "camelCase")]
    Presence {
        canvas_id: String,
        payload: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    Resume {
        canvas_id: String,
        last_ack_seq: i64,
    },
}

// ---------------------------------------------------------------------------
// Server → Client messages.
// ---------------------------------------------------------------------------

/// Downstream messages. Internally tagged on `type`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage {
    /// Handshake reply: either a region `snapshot` (fresh subscribe) or a
    /// `delta_since` cursor (resume), plus the current server seq/revision.
    #[serde(rename_all = "camelCase")]
    Welcome {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot: Option<ObjectScene>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta_since: Option<i64>,
        seq: i64,
        revision: i64,
    },
    #[serde(rename_all = "camelCase")]
    Ack {
        op_ids: Vec<OpId>,
        seq: i64,
        revision: i64,
    },
    #[serde(rename_all = "camelCase")]
    Patch { ops: Vec<WireOp>, seq: i64 },
    #[serde(rename_all = "camelCase")]
    Presence { payload: serde_json::Value },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

// ---------------------------------------------------------------------------
// Logical transport channels.
// ---------------------------------------------------------------------------

/// The two logical channels multiplexed over a single transport (PC8). Ops/acks
/// ride the reliable ordered channel; presence rides the lossy ephemeral one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    #[serde(rename = "reliable_ordered")]
    ReliableOrdered,
    #[serde(rename = "ephemeral_besteffort")]
    EphemeralBestEffort,
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::ReliableOrdered => "reliable_ordered",
            Channel::EphemeralBestEffort => "ephemeral_besteffort",
        }
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_wire_op() -> WireOp {
        WireOp {
            op_id: OpId {
                client_id: "client-a".to_string(),
                local_seq: 7,
            },
            object_id: "node-1".to_string(),
            kind: "move-card".to_string(),
            prop_delta: json!({ "x": 12.0, "y": 34.0 }),
            base_revision: 41,
            actor: "user-42".to_string(),
            ts: "2026-06-07T00:00:00.000Z".to_string(),
        }
    }

    fn round_trip_client(msg: &ClientMessage) {
        let s = serde_json::to_string(msg).unwrap();
        let back: ClientMessage = serde_json::from_str(&s).unwrap();
        assert_eq!(*msg, back);
    }

    fn round_trip_server(msg: &ServerMessage) {
        let s = serde_json::to_string(msg).unwrap();
        let back: ServerMessage = serde_json::from_str(&s).unwrap();
        assert_eq!(*msg, back);
    }

    #[test]
    fn region_whole_canvas_omits_bbox() {
        let r = Region {
            canvas_id: "c1".to_string(),
            bbox: None,
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({ "canvasId": "c1" }));
        let back: Region = serde_json::from_value(v).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn region_with_bbox_round_trips() {
        let r = Region {
            canvas_id: "c1".to_string(),
            bbox: Some(Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 200.0,
            }),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: Region = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn op_id_uses_camel_case_keys() {
        let id = OpId {
            client_id: "client-a".to_string(),
            local_seq: 9,
        };
        let v: serde_json::Value = serde_json::to_value(&id).unwrap();
        assert_eq!(v, json!({ "clientId": "client-a", "localSeq": 9 }));
    }

    #[test]
    fn wire_op_round_trips_with_camel_case_keys() {
        let op = sample_wire_op();
        let v: serde_json::Value = serde_json::to_value(&op).unwrap();
        assert_eq!(v["opId"]["clientId"], json!("client-a"));
        assert_eq!(v["objectId"], json!("node-1"));
        assert_eq!(v["propDelta"], json!({ "x": 12.0, "y": 34.0 }));
        assert_eq!(v["baseRevision"], json!(41));
        let back: WireOp = serde_json::from_value(v).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn client_hello_round_trips_and_tags() {
        let msg = ClientMessage::Hello {
            canvas_id: "c1".to_string(),
            region: Region {
                canvas_id: "c1".to_string(),
                bbox: None,
            },
            last_ack_seq: 0,
        };
        round_trip_client(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("hello"));
        assert_eq!(v["lastAckSeq"], json!(0));
    }

    #[test]
    fn client_subscribe_round_trips() {
        let msg = ClientMessage::Subscribe {
            canvas_id: "c1".to_string(),
            region: Region {
                canvas_id: "c1".to_string(),
                bbox: Some(Bounds {
                    x: -50.0,
                    y: -50.0,
                    width: 100.0,
                    height: 100.0,
                }),
            },
        };
        round_trip_client(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("subscribe"));
    }

    #[test]
    fn client_ops_round_trips() {
        let msg = ClientMessage::Ops {
            ops: vec![sample_wire_op()],
        };
        round_trip_client(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("ops"));
    }

    #[test]
    fn client_presence_round_trips() {
        let msg = ClientMessage::Presence {
            canvas_id: "c1".to_string(),
            payload: json!({ "cursor": { "x": 1.0, "y": 2.0 } }),
        };
        round_trip_client(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("presence"));
    }

    #[test]
    fn client_resume_round_trips() {
        let msg = ClientMessage::Resume {
            canvas_id: "c1".to_string(),
            last_ack_seq: 128,
        };
        round_trip_client(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("resume"));
    }

    #[test]
    fn server_welcome_snapshot_round_trips() {
        let snapshot = ObjectScene {
            scene_version: 3,
            updated_at: "2026-06-07T00:00:00.000Z".to_string(),
            ..Default::default()
        };
        let msg = ServerMessage::Welcome {
            snapshot: Some(snapshot),
            delta_since: None,
            seq: 10,
            revision: 3,
        };
        round_trip_server(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("welcome"));
        assert!(v.get("deltaSince").is_none());
        assert!(v.get("snapshot").is_some());
    }

    #[test]
    fn server_welcome_delta_round_trips() {
        let msg = ServerMessage::Welcome {
            snapshot: None,
            delta_since: Some(99),
            seq: 120,
            revision: 17,
        };
        round_trip_server(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("welcome"));
        assert_eq!(v["deltaSince"], json!(99));
        assert!(v.get("snapshot").is_none());
    }

    #[test]
    fn server_ack_round_trips() {
        let msg = ServerMessage::Ack {
            op_ids: vec![OpId {
                client_id: "client-a".to_string(),
                local_seq: 7,
            }],
            seq: 11,
            revision: 4,
        };
        round_trip_server(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("ack"));
        assert_eq!(v["opIds"][0]["localSeq"], json!(7));
    }

    #[test]
    fn server_patch_round_trips() {
        let msg = ServerMessage::Patch {
            ops: vec![sample_wire_op()],
            seq: 12,
        };
        round_trip_server(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("patch"));
    }

    #[test]
    fn server_presence_round_trips() {
        let msg = ServerMessage::Presence {
            payload: json!({ "users": ["user-1", "user-2"] }),
        };
        round_trip_server(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("presence"));
    }

    #[test]
    fn server_error_round_trips() {
        let msg = ServerMessage::Error {
            message: "rejected: stale base revision".to_string(),
        };
        round_trip_server(&msg);
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], json!("error"));
        assert_eq!(v["message"], json!("rejected: stale base revision"));
    }

    #[test]
    fn channel_as_str_and_serde() {
        assert_eq!(Channel::ReliableOrdered.as_str(), "reliable_ordered");
        assert_eq!(Channel::EphemeralBestEffort.as_str(), "ephemeral_besteffort");
        assert_eq!(
            serde_json::to_value(Channel::ReliableOrdered).unwrap(),
            json!("reliable_ordered")
        );
        assert_eq!(
            serde_json::to_value(Channel::EphemeralBestEffort).unwrap(),
            json!("ephemeral_besteffort")
        );
        let back: Channel = serde_json::from_value(json!("ephemeral_besteffort")).unwrap();
        assert_eq!(back, Channel::EphemeralBestEffort);
    }
}
