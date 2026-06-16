//! Shared fixtures for the client-runtime tests. Each test binary uses a
//! different subset, so per-binary unused-helper warnings are expected.
#![allow(dead_code)]

use shape_client_runtime::outbox::OutboxEntry;
use shape_client_runtime::sync_engine::EngineTransport;
use shape_scene_core::object::model::{
    FillRule, Geometry, Object, ObjectScene, Transform3x3,
};
use shape_scene_core::object::op::ObjectOp;

pub fn empty_scene() -> ObjectScene {
    ObjectScene::default()
}

pub fn translate(tx: f64, ty: f64) -> Transform3x3 {
    Transform3x3::translate(tx, ty)
}

/// A closed quad that validates under op-apply.
pub fn rect(id: &str, order: &str) -> Object {
    Object {
        id: id.to_string(),
        parent: None,
        order: order.to_string(),
        transform: Transform3x3::IDENTITY,
        warp: None,
        geometry: Geometry {
            path_string: "M 0 0 L 80 0 L 80 40 L 0 40 Z".to_string(),
            fill_rule: FillRule::default(),
            subpaths: Vec::new(),
        },
        fill: None,
        stroke: None,
        text: None,
        anchors: Vec::new(),
        layout: None,
        sizing: None,
        clip: None,
        comments: Vec::new(),
        tags: Vec::new(),
        name: None,
        hidden: false,
        locked: false,
        component_of: None,
        content: None,
        meta: None,
    }
}

pub fn insert(object: Object) -> ObjectOp {
    ObjectOp::InsertObject { object }
}

pub fn move_op(id: &str, x: f64, y: f64) -> ObjectOp {
    ObjectOp::SetTransform {
        id: id.to_string(),
        transform: translate(x, y),
    }
}

pub fn text_op(id: &str, value: &str) -> ObjectOp {
    use shape_scene_core::object::model::{Text, TextRun};
    ObjectOp::SetText {
        id: id.to_string(),
        text: Some(Text {
            runs: vec![TextRun {
                text: value.to_string(),
                color: None,
                size: None,
                bold: false,
                italic: false,
                font: None,
            }],
            align: Default::default(),
            valign: Default::default(),
        }),
    }
}

pub fn object_ids(scene: &ObjectScene) -> Vec<String> {
    scene.objects.iter().map(|o| o.id.clone()).collect()
}

pub fn object_transform(scene: &ObjectScene, id: &str) -> Option<Transform3x3> {
    scene.objects.iter().find(|o| o.id == id).map(|o| o.transform.clone())
}

pub fn object_text(scene: &ObjectScene, id: &str) -> Option<String> {
    scene
        .objects
        .iter()
        .find(|o| o.id == id)
        .and_then(|o| o.text.as_ref())
        .and_then(|t| t.runs.first())
        .map(|r| r.text.clone())
}

/// A mock `EngineTransport` that records flushed batches.
#[derive(Default)]
pub struct CaptureTransport {
    pub batches: Vec<Vec<OutboxEntry>>,
}

impl CaptureTransport {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn flat(&self) -> Vec<OutboxEntry> {
        self.batches.iter().flatten().cloned().collect()
    }
}

impl EngineTransport for CaptureTransport {
    fn send_envelopes(&mut self, entries: &[OutboxEntry]) {
        self.batches.push(entries.to_vec());
    }
}

/// A fixed, monotonic clock-stamp source: `t0`, `t1`, ….
pub struct FixedNow {
    n: usize,
}
impl FixedNow {
    pub fn new() -> Self {
        Self { n: 0 }
    }
    pub fn next(&mut self) -> String {
        let s = format!("t{}", self.n);
        self.n += 1;
        s
    }
}

pub fn op_id(client: &str, seq: i64) -> shape_scene_core::wire::OpId {
    shape_scene_core::wire::OpId {
        client_id: client.to_string(),
        local_seq: seq,
    }
}
