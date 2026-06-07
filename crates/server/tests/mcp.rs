//! OB4.1 integration tests for the object-native MCP tool surface, driving the
//! rmcp tool handler fns directly (the same fns the streamable-HTTP transport
//! dispatches — no socket, no JSON-RPC framing).

use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;
use shape_server::mcp::{
    AddCommentArgs, CanvasOnlyArgs, CreateObjectArgs, GetObjectArgs, PatchObjectArgs, QueryArgs,
    TagObjectArgs,
};
use shape_server::{CanvasRegistry, SceneMcp};

/// Build a SceneMcp over a fresh in-memory canvas registry.
fn mcp_instance() -> SceneMcp {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    SceneMcp::new(canvases)
}

/// Extract the JSON text from a tool result's single text content block.
fn result_json(result: &rmcp::model::CallToolResult) -> Value {
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content block");
    serde_json::from_str(&text).expect("tool result is JSON")
}

#[test]
fn tool_router_lists_object_tools() {
    let tools = SceneMcp::tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in [
        "list_objects",
        "get_object",
        "create_object",
        "patch_object",
        "tag_object",
        "add_comment",
        "query",
        "export",
        "set_selection",
    ] {
        assert!(names.contains(&expected), "missing tool {expected} in {names:?}");
    }
    for tool in &tools {
        assert!(
            !tool.input_schema.is_empty(),
            "tool {} has no input schema",
            tool.name
        );
    }
}

#[tokio::test]
async fn create_object_then_list_and_get() {
    let mcp = mcp_instance();

    let before = mcp
        .list_objects(Parameters(CanvasOnlyArgs::default()))
        .await
        .unwrap();
    assert_eq!(result_json(&before)["objects"].as_array().unwrap().len(), 0);

    let created = mcp
        .create_object(Parameters(CreateObjectArgs {
            id: Some("rect-1".into()),
            order: Some("a0".into()),
            shape: "rect".into(),
            x: 100.0,
            y: 50.0,
            width: Some(160),
            height: Some(100),
            text: Some("Hello".into()),
            style: Some("decision".into()),
            tags: vec!["t-blue".into()],
            canvas_id: None,
        }))
        .await
        .expect("create_object");
    let created = result_json(&created);
    assert_eq!(created["object"]["id"], "rect-1");

    // The change is durable: list + get see it.
    let listed = mcp
        .list_objects(Parameters(CanvasOnlyArgs::default()))
        .await
        .unwrap();
    let listed = result_json(&listed);
    let objects = listed["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0]["id"], "rect-1");
    assert_eq!(objects[0]["kind"], "shape", "a rect is a descriptive 'shape'");

    let got = mcp
        .get_object(Parameters(GetObjectArgs {
            id: "rect-1".into(),
            canvas_id: None,
        }))
        .await
        .expect("get_object");
    assert_eq!(result_json(&got)["object"]["id"], "rect-1");
}

#[tokio::test]
async fn create_object_rejects_invalid_spec_with_error() {
    let mcp = mcp_instance();
    let err = mcp
        .create_object(Parameters(CreateObjectArgs {
            id: Some("bad".into()),
            order: Some("a0".into()),
            shape: "rect".into(),
            x: 0.0,
            y: 0.0,
            width: Some(0),
            height: Some(10),
            text: None,
            style: None,
            tags: vec![],
            canvas_id: None,
        }))
        .await
        .expect_err("zero-width rect is an error");
    assert!(err.message.contains("positive"), "got {:?}", err.message);
}

#[tokio::test]
async fn patch_object_moves_and_resizes() {
    let mcp = mcp_instance();
    mcp.create_object(Parameters(CreateObjectArgs {
        id: Some("r".into()),
        order: Some("a0".into()),
        shape: "rect".into(),
        x: 0.0,
        y: 0.0,
        width: Some(80),
        height: Some(40),
        text: None,
        style: None,
        tags: vec![],
        canvas_id: None,
    }))
    .await
    .unwrap();

    let patched = mcp
        .patch_object(Parameters(PatchObjectArgs {
            id: "r".into(),
            text: Some("New".into()),
            style: None,
            x: Some(10.0),
            y: Some(20.0),
            width: Some(120),
            height: Some(60),
            canvas_id: None,
        }))
        .await
        .expect("patch_object");
    let object = result_json(&patched)["object"].clone();
    // Transform3x3 is `#[serde(transparent)]` over its 3x3 array, so the world
    // placement is the translation column [0][2], [1][2].
    assert_eq!(object["transform"][0][2], 10.0);
    assert_eq!(object["transform"][1][2], 20.0);
}

#[tokio::test]
async fn tag_object_then_query_by_tag() {
    let mcp = mcp_instance();
    for (id, tag) in [("a", "keep"), ("b", "other")] {
        mcp.create_object(Parameters(CreateObjectArgs {
            id: Some(id.into()),
            order: Some(format!("a{id}")),
            shape: "rect".into(),
            x: 0.0,
            y: 0.0,
            width: Some(40),
            height: Some(40),
            text: None,
            style: None,
            tags: vec![tag.into()],
            canvas_id: None,
        }))
        .await
        .unwrap();
    }

    // Retag b to also carry "keep".
    mcp.tag_object(Parameters(TagObjectArgs {
        id: "b".into(),
        tags: vec!["keep".into()],
        canvas_id: None,
    }))
    .await
    .unwrap();

    let res = mcp
        .query(Parameters(QueryArgs {
            tags: vec!["keep".into()],
            connected_to: None,
            region: None,
            canvas_id: None,
        }))
        .await
        .expect("query");
    let ids: Vec<String> = result_json(&res)["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["a".to_string(), "b".to_string()], "both carry 'keep'");
}

#[tokio::test]
async fn add_comment_attaches_to_object() {
    let mcp = mcp_instance();
    mcp.create_object(Parameters(CreateObjectArgs {
        id: Some("o1".into()),
        order: Some("a0".into()),
        shape: "rect".into(),
        x: 0.0,
        y: 0.0,
        width: Some(80),
        height: Some(40),
        text: None,
        style: None,
        tags: vec![],
        canvas_id: None,
    }))
    .await
    .unwrap();

    let res = mcp
        .add_comment(Parameters(AddCommentArgs {
            id: "o1".into(),
            comment_id: Some("c-1".into()),
            author: Some("agent".into()),
            body: "looks good".into(),
            node_index: None,
            canvas_id: None,
        }))
        .await
        .expect("add_comment");
    let res = result_json(&res);
    assert_eq!(res["commentId"], "c-1");
    assert_eq!(res["object"]["comments"].as_array().unwrap().len(), 1);
    assert_eq!(res["object"]["comments"][0]["body"], "looks good");
}

#[tokio::test]
async fn export_of_two_connected_objects_mentions_both() {
    let mcp = mcp_instance();
    for (id, x, text) in [("src", 0.0, "Source"), ("dst", 300.0, "Dest")] {
        mcp.create_object(Parameters(CreateObjectArgs {
            id: Some(id.into()),
            order: Some(format!("a{id}")),
            shape: "rect".into(),
            x,
            y: 0.0,
            width: Some(80),
            height: Some(40),
            text: Some(text.into()),
            style: None,
            tags: vec![],
            canvas_id: None,
        }))
        .await
        .unwrap();
    }

    let res = mcp
        .export(Parameters(shape_server::mcp::ExportArgs {
            scope_ids: vec![],
            export_type: Some("digest".into()),
            canvas_id: None,
        }))
        .await
        .expect("export");
    let content = result_json(&res)["content"].as_str().unwrap().to_string();
    assert!(content.contains("Source"), "digest mentions source: {content}");
    assert!(content.contains("Dest"), "digest mentions dest: {content}");
}
