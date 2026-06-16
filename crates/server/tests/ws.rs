//! WebSocket transport integration tests: bind the full router on an ephemeral
//! port and drive it with a real `tokio-tungstenite` client. Each test gets its
//! own in-memory registry so canvases never leak state.

use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use shape_server::{build_router_with_mcp, CanvasRegistry, Config};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

fn test_config() -> Config {
    Config {
        host: "127.0.0.1".to_string(),
        port: 0,
        client_dir: std::env::temp_dir().join("shape_server_ws_test_no_client_dir"),
    }
}

async fn spawn_server() -> SocketAddr {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let router = build_router_with_mcp(&test_config(), canvases);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: SocketAddr) -> WsStream {
    let url = format!("ws://{addr}/ws");
    let (ws, _resp) = connect_async(url).await.expect("ws upgrade");
    ws
}

async fn send_json(ws: &mut WsStream, v: Value) {
    ws.send(Message::Text(v.to_string())).await.unwrap();
}

async fn recv_json(ws: &mut WsStream) -> Value {
    loop {
        match ws.next().await.expect("socket open").expect("frame ok") {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected non-text frame: {other:?}"),
        }
    }
}

async fn recv_json_of(ws: &mut WsStream, ty: &str) -> Value {
    loop {
        let v = recv_json(ws).await;
        if v["type"] == ty {
            return v;
        }
    }
}

/// `Transform3x3` is `#[serde(transparent)]`, so the wire transform is a bare 3x3
/// array, not `{ "m": [...] }`.
fn insert_delta(id: &str, order: &str, x: f64, y: f64) -> Value {
    json!({
        "kind": "insert-object",
        "object": {
            "id": id,
            "order": order,
            "transform": [[1.0, 0.0, x], [0.0, 1.0, y], [0.0, 0.0, 1.0]],
            "geometry": { "d": "M 0 0 L 80 0 L 80 40 L 0 40 Z", "fillRule": "evenOdd" }
        }
    })
}

fn move_delta(id: &str, x: f64, y: f64) -> Value {
    json!({
        "kind": "set-transform",
        "id": id,
        "transform": [[1.0, 0.0, x], [0.0, 1.0, y], [0.0, 0.0, 1.0]]
    })
}

fn region(canvas_id: &str, bbox: Option<(f64, f64, f64, f64)>) -> Value {
    match bbox {
        None => json!({ "canvasId": canvas_id }),
        Some((x, y, w, h)) => json!({
            "canvasId": canvas_id,
            "bbox": { "x": x, "y": y, "width": w, "height": h }
        }),
    }
}

fn wire_op(client_id: &str, local_seq: i64, base_revision: i64, object_id: &str, kind: &str, delta: Value) -> Value {
    json!({
        "opId": { "clientId": client_id, "localSeq": local_seq },
        "objectId": object_id,
        "kind": kind,
        "propDelta": delta,
        "baseRevision": base_revision,
        "actor": client_id,
        "ts": "1970-01-01T00:00:00Z",
    })
}

#[tokio::test]
async fn hello_yields_welcome_then_ops_ack_with_increasing_seq() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    send_json(&mut ws, json!({ "type": "hello", "canvasId": "c-ws", "lastAckSeq": 0 })).await;
    let welcome = recv_json(&mut ws).await;
    assert_eq!(welcome["type"], "welcome");
    assert!(welcome["scene"].is_object(), "welcome carries an object scene");
    assert_eq!(welcome["scene"]["objects"].as_array().unwrap().len(), 0);
    assert_eq!(welcome["revision"], json!(0));

    send_json(
        &mut ws,
        json!({ "type": "ops", "ops": [wire_op("user-1", 1, 0, "o1", "insert-object", insert_delta("o1", "a0", 0.0, 0.0))] }),
    )
    .await;
    let ack1 = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack1["seq"], json!(1), "first apply is seq 1");
    assert_eq!(
        ack1["opIds"][0],
        json!({ "clientId": "user-1", "localSeq": 1 }),
        "ack echoes the op's id"
    );

    send_json(
        &mut ws,
        json!({ "type": "ops", "ops": [wire_op("user-1", 2, 1, "o2", "insert-object", insert_delta("o2", "a1", 5.0, 5.0))] }),
    )
    .await;
    let ack2 = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack2["seq"], json!(2), "second apply is seq 2, increasing");
}

#[tokio::test]
async fn duplicate_op_id_is_idempotent_reacks_same_seq() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    send_json(&mut ws, json!({ "type": "hello", "canvasId": "c-dedup", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws).await["type"], "welcome");

    let dup = wire_op("user-1", 1, 0, "o1", "insert-object", insert_delta("o1", "a0", 0.0, 0.0));
    send_json(&mut ws, json!({ "type": "ops", "ops": [dup.clone()] })).await;
    let ack1 = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack1["seq"], json!(1));

    send_json(&mut ws, json!({ "type": "ops", "ops": [dup] })).await;
    let ack_dup = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack_dup["seq"], json!(1), "duplicate op re-acks the original seq");

    send_json(
        &mut ws,
        json!({ "type": "ops", "ops": [wire_op("user-1", 2, 1, "o2", "insert-object", insert_delta("o2", "a1", 1.0, 1.0))] }),
    )
    .await;
    let ack2 = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack2["seq"], json!(2), "new op is seq 2 (duplicate did not bump)");

    send_json(&mut ws, json!({ "type": "resume", "canvasId": "c-dedup", "lastAckSeq": 0 })).await;
    let welcome = recv_json_of(&mut ws, "welcome").await;
    assert_eq!(welcome["scene"]["objects"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn rejected_ops_yield_rejected_message() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    send_json(&mut ws, json!({ "type": "hello", "canvasId": "c-reject", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws).await["type"], "welcome");

    send_json(
        &mut ws,
        json!({ "type": "ops", "ops": [wire_op("user-1", 1, 0, "missing", "set-transform", move_delta("missing", 1.0, 1.0))] }),
    )
    .await;
    let reply = recv_json_of(&mut ws, "rejected").await;
    assert!(
        !reply["errors"].as_array().unwrap().is_empty(),
        "rejection carries errors"
    );
    assert_eq!(
        reply["opIds"][0],
        json!({ "clientId": "user-1", "localSeq": 1 }),
        "rejection echoes the op's id"
    );
}

#[tokio::test]
async fn second_client_receives_patch_broadcast_from_first() {
    let addr = spawn_server().await;

    let mut ws1 = connect(addr).await;
    send_json(&mut ws1, json!({ "type": "hello", "canvasId": "c-multi", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws1).await["type"], "welcome");

    let mut ws2 = connect(addr).await;
    send_json(&mut ws2, json!({ "type": "hello", "canvasId": "c-multi", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws2).await["type"], "welcome");

    send_json(
        &mut ws1,
        json!({ "type": "ops", "ops": [wire_op("user-1", 1, 0, "o1", "insert-object", insert_delta("o1", "a0", 0.0, 0.0))] }),
    )
    .await;
    let ack = recv_json_of(&mut ws1, "ack").await;
    assert_eq!(ack["seq"], json!(1));

    let patch = recv_json_of(&mut ws2, "patch").await;
    assert_eq!(patch["seq"], json!(1), "patch carries the applied seq");
    assert_eq!(patch["ops"][0]["kind"], json!("insert-object"));
    assert_eq!(patch["ops"][0]["propDelta"]["object"]["id"], json!("o1"));
}

#[tokio::test]
async fn presence_fans_out_to_other_client_best_effort() {
    let addr = spawn_server().await;

    let mut ws1 = connect(addr).await;
    send_json(&mut ws1, json!({ "type": "hello", "canvasId": "c-presence", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws1).await["type"], "welcome");

    let mut ws2 = connect(addr).await;
    send_json(&mut ws2, json!({ "type": "hello", "canvasId": "c-presence", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws2).await["type"], "welcome");

    send_json(
        &mut ws1,
        json!({
            "type": "presence",
            "canvasId": "c-presence",
            "payload": { "cursor": { "x": 12.0, "y": 34.0 }, "userId": "user-1" },
        }),
    )
    .await;

    let frame = recv_json(&mut ws2).await;
    assert_eq!(frame["type"], "presence", "got {frame}");
    assert_eq!(frame["payload"]["userId"], json!("user-1"));
    assert_eq!(frame["payload"]["cursor"]["x"], json!(12.0));

    assert!(
        try_recv_json_of(&mut ws1, "presence", 300).await.is_none(),
        "a connection does not receive its own presence frame"
    );
}

#[tokio::test]
async fn non_hello_first_message_errors() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    send_json(&mut ws, json!({ "type": "ops", "ops": [] })).await;
    let reply = recv_json(&mut ws).await;
    assert_eq!(reply["type"], "error", "got {reply}");
}

#[tokio::test]
async fn feature_comment_upsert_round_trips_over_ws() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    send_json(&mut ws, json!({ "type": "hello", "canvasId": "c-feature", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws).await["type"], "welcome");

    send_json(
        &mut ws,
        json!({ "type": "ops", "ops": [wire_op("u", 1, 0, "o1", "insert-object", insert_delta("o1", "a0", 0.0, 0.0))] }),
    )
    .await;
    assert_eq!(recv_json_of(&mut ws, "ack").await["seq"], json!(1));

    // `FeatureRequest` serde tags variants on `feature` (camelCase) but leaves the
    // variant fields snake_case, so the wire uses `canvas_id` / `object_id`.
    send_json(
        &mut ws,
        json!({
            "type": "feature",
            "request": {
                "feature": "commentUpsert",
                "canvas_id": "c-feature",
                "object_id": "o1",
                "comment": { "id": "c-1", "author": "jayden", "body": "looks good", "resolved": false }
            }
        }),
    )
    .await;
    let resp = recv_json_of(&mut ws, "feature").await;
    assert_eq!(resp["response"]["feature"], json!("commentUpserted"));
    assert_eq!(resp["response"]["object_id"], json!("o1"));
    assert_eq!(resp["response"]["comment_id"], json!("c-1"));

    send_json(&mut ws, json!({ "type": "resume", "canvasId": "c-feature", "lastAckSeq": 0 })).await;
    let welcome = recv_json_of(&mut ws, "welcome").await;
    let obj = &welcome["scene"]["objects"][0];
    assert_eq!(obj["comments"].as_array().unwrap().len(), 1, "comment persisted");
}

use std::time::Duration;

async fn try_recv_json_of(ws: &mut WsStream, ty: &str, ms: u64) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let v: Value = serde_json::from_str(&t).unwrap();
                if v["type"] == ty {
                    return Some(v);
                }
            }
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(Some(Ok(_))) | Ok(Some(Err(_))) | Ok(None) => return None,
            Err(_) => return None,
        }
    }
}

#[tokio::test]
async fn region_windowing_filters_welcome_and_fanout() {
    let addr = spawn_server().await;
    let canvas = "c-windowing";

    let mut seed = connect(addr).await;
    send_json(&mut seed, json!({ "type": "hello", "canvasId": canvas, "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut seed).await["type"], "welcome");

    let seed_ops = [
        ("oA1", "a0", 0.0, 0.0),
        ("oA2", "a1", 200.0, 10.0),
        ("oB1", "a2", 100_000.0, 100_000.0),
    ];
    for (i, (id, order, x, y)) in seed_ops.iter().enumerate() {
        let ls = i64::try_from(i + 1).unwrap();
        send_json(
            &mut seed,
            json!({ "type": "ops", "ops": [wire_op("seed", ls, ls - 1, id, "insert-object", insert_delta(id, order, *x, *y))] }),
        )
        .await;
        assert_eq!(recv_json_of(&mut seed, "ack").await["seq"], json!(ls), "seed op {ls} acked");
    }

    let win_a = Some((-50.0, -50.0, 600.0, 500.0));
    let win_b = Some((99_900.0, 99_900.0, 700.0, 600.0));

    let mut a = connect(addr).await;
    send_json(
        &mut a,
        json!({ "type": "hello", "canvasId": canvas, "region": region(canvas, win_a), "lastAckSeq": 0 }),
    )
    .await;
    let welcome_a = recv_json_of(&mut a, "welcome").await;
    let a_ids: Vec<&str> = welcome_a["scene"]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    let mut a_ids_sorted = a_ids.clone();
    a_ids_sorted.sort();
    assert_eq!(a_ids_sorted, vec!["oA1", "oA2"], "region-A welcome has A's objects");

    let mut b = connect(addr).await;
    send_json(
        &mut b,
        json!({ "type": "hello", "canvasId": canvas, "region": region(canvas, win_b), "lastAckSeq": 0 }),
    )
    .await;
    let welcome_b = recv_json_of(&mut b, "welcome").await;
    assert_eq!(welcome_b["scene"]["objects"][0]["id"], json!("oB1"), "region-B welcome is oB1");

    send_json(
        &mut seed,
        json!({ "type": "ops", "ops": [wire_op("seed", 4, 3, "oB2", "insert-object", insert_delta("oB2", "a3", 100_200.0, 100_010.0))] }),
    )
    .await;
    assert_eq!(recv_json_of(&mut seed, "ack").await["seq"], json!(4), "region-B op acked");

    let patch_b = try_recv_json_of(&mut b, "patch", 1000)
        .await
        .expect("region-B subscriber receives the in-region patch");
    assert_eq!(patch_b["ops"][0]["propDelta"]["object"]["id"], json!("oB2"), "B sees the region-B object");

    assert!(
        try_recv_json_of(&mut a, "patch", 300).await.is_none(),
        "region-A subscriber does not receive the region-B op"
    );

    send_json(
        &mut a,
        json!({ "type": "subscribe", "canvasId": canvas, "region": region(canvas, win_b) }),
    )
    .await;
    let resnap = recv_json_of(&mut a, "welcome").await;
    let mut ids: Vec<&str> = resnap["scene"]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["oB1", "oB2"], "after re-subscribe, A's snapshot has B's objects");

    let mut all = connect(addr).await;
    send_json(
        &mut all,
        json!({ "type": "hello", "canvasId": canvas, "region": region(canvas, None), "lastAckSeq": 0 }),
    )
    .await;
    let welcome_all = recv_json_of(&mut all, "welcome").await;
    assert_eq!(welcome_all["scene"]["objects"].as_array().unwrap().len(), 4, "None sees all four objects");
}

#[tokio::test]
async fn author_does_not_receive_own_op_but_peer_does() {
    let addr = spawn_server().await;
    let canvas = "c-selfskip";

    let mut author = connect(addr).await;
    send_json(
        &mut author,
        json!({ "type": "hello", "canvasId": canvas, "userId": "user-A", "lastAckSeq": 0 }),
    )
    .await;
    assert_eq!(recv_json(&mut author).await["type"], "welcome");

    let mut peer = connect(addr).await;
    send_json(
        &mut peer,
        json!({ "type": "hello", "canvasId": canvas, "userId": "user-B", "lastAckSeq": 0 }),
    )
    .await;
    assert_eq!(recv_json(&mut peer).await["type"], "welcome");

    send_json(
        &mut author,
        json!({ "type": "ops", "ops": [wire_op("user-A", 1, 0, "o1", "insert-object", insert_delta("o1", "a0", 0.0, 0.0))] }),
    )
    .await;
    let ack = recv_json_of(&mut author, "ack").await;
    assert_eq!(ack["seq"], json!(1), "author gets the ack");

    assert!(
        try_recv_json_of(&mut author, "patch", 400).await.is_none(),
        "originator is not echoed its own applied op"
    );

    let patch = try_recv_json_of(&mut peer, "patch", 1000)
        .await
        .expect("peer receives the author's op");
    assert_eq!(patch["seq"], json!(1), "peer's patch carries the applied seq");
    assert_eq!(patch["ops"][0]["propDelta"]["object"]["id"], json!("o1"), "peer sees the created object");
}
