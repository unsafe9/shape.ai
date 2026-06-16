//! Integration tests for the MCP tool surface, driving the rmcp tool handler fns
//! directly (no socket, no JSON-RPC framing).

use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;
use shape_server::mcp::{
    AddCommentArgs, CanvasOnlyArgs, CreateObjectArgs, GetObjectArgs, PatchObjectArgs, QueryArgs,
    TagObjectArgs,
};
use shape_server::{CanvasRegistry, ExtensionRegistry, SceneMcp};

fn mcp_instance() -> SceneMcp {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    SceneMcp::new(canvases)
}

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
    let tools = mcp_instance().tool_definitions();
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

/// The MCP-register seam: `tool_definitions` (what `list_tools` returns) overlays
/// EVERY extension's namespaced tools on top of the core object tools. This is the
/// falsifiable guard for the per-extension registration path — it FAILS if the
/// facade ever drops the extension tool group (e.g. reverting to the macro-
/// generated `#[tool_handler]` that sees only the core router).
#[test]
fn tool_definitions_advertise_namespaced_extension_tools() {
    let names: Vec<String> = mcp_instance()
        .tool_definitions()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    // The two reference extensions, namespaced `ext_<name>_<tool>`.
    for expected in [
        "ext_kanban_list_board",
        "ext_kanban_add_column",
        "ext_kanban_add_card",
        "ext_diagram_list",
        "ext_diagram_add_node",
        "ext_diagram_connect",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing extension tool {expected} in {names:?}"
        );
    }
    // The core tools are still there alongside them (one facade, both groups).
    assert!(names.iter().any(|n| n == "create_object"));
}

/// The MCP-register seam end to end: a namespaced `ext_*` author call dispatches
/// through the registry, whose author path funnels through the SAME `ActorHandle::
/// apply_op` (so the board's objects land on the canvas the core tools also see),
/// and a subsequent read reflects the authored model. Driving the real registry +
/// a real actor handle is what makes this falsifiable against a broken path.
#[tokio::test]
async fn extension_author_and_read_funnel_through_the_one_apply_op() {
    use shape_scene_core::CanvasId;
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let handle = canvases.get_or_spawn(&CanvasId::from("default")).await.unwrap();
    let registry = ExtensionRegistry::with_builtins();

    // Author a kanban column + card via the namespaced extension tools.
    registry
        .dispatch(&handle, "ext_kanban_add_column", &serde_json::json!({ "id": "todo", "title": "To do" }))
        .await
        .expect("add_column dispatches");
    registry
        .dispatch(&handle, "ext_kanban_add_card", &serde_json::json!({ "id": "k1", "column": "todo", "title": "First" }))
        .await
        .expect("add_card dispatches");

    // The domain read reflects the authored model (one column, one card).
    let listed = registry
        .dispatch(&handle, "ext_kanban_list_board", &serde_json::json!({}))
        .await
        .expect("list_board dispatches");
    assert_eq!(listed["columns"][0]["id"], "todo");
    assert_eq!(listed["columns"][0]["cards"].as_array().unwrap().len(), 1);

    // The export landed real objects on the SAME canvas the core tools see: the
    // card rect is a normal scene object the extension tagged + keyed by domain key.
    let scene = handle.get_scene().await;
    assert!(
        scene.get("ext-kanban-card:k1").is_some(),
        "the authored card object rode the one op-apply onto the canvas"
    );
}

/// The graph-shaped reference's MCP round-trip through the server boundary: author
/// nodes + edges via the namespaced `ext_diagram_*` tools (the same registry +
/// real actor a live `/mcp` call uses), then assert the core `connection_graph`
/// over the resulting scene EQUALS the authored edge set. This proves a relational
/// domain rides anchors all the way through the one op-apply — and is falsifiable
/// against a broken export/dispatch/reconcile (it fails if the edges don't land as
/// real connector objects the core graph can see).
#[tokio::test]
async fn diagram_author_lands_edges_the_core_connection_graph_sees() {
    use shape_scene_core::object::connection_graph;
    use shape_scene_core::CanvasId;
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let handle = canvases.get_or_spawn(&CanvasId::from("default")).await.unwrap();
    let registry = ExtensionRegistry::with_builtins();

    for (id, x) in [("a", 0.0), ("b", 300.0), ("c", 600.0)] {
        registry
            .dispatch(&handle, "ext_diagram_add_node", &serde_json::json!({ "id": id, "x": x, "y": 0.0 }))
            .await
            .expect("add_node dispatches");
    }
    registry
        .dispatch(&handle, "ext_diagram_connect", &serde_json::json!({ "id": "e1", "from": "a", "to": "b" }))
        .await
        .expect("connect a->b dispatches");
    registry
        .dispatch(&handle, "ext_diagram_connect", &serde_json::json!({ "id": "e2", "from": "b", "to": "c" }))
        .await
        .expect("connect b->c dispatches");

    // The exported scene's connection graph (derived by the core over real anchor
    // objects) equals the authored edges — the relational invariant, server-side.
    let scene = handle.get_scene().await;
    let mut graph = connection_graph(&scene);
    graph.sort();
    let mut want = vec![
        ("ext-diagram-node:a".to_string(), "ext-diagram-node:b".to_string()),
        ("ext-diagram-node:b".to_string(), "ext-diagram-node:c".to_string()),
    ];
    want.sort();
    assert_eq!(graph, want, "core connection graph == authored edges (a-b, b-c)");

    // The domain read reflects the authored model over the SAME canvas.
    let listed = registry
        .dispatch(&handle, "ext_diagram_list", &serde_json::json!({}))
        .await
        .expect("diagram list dispatches");
    assert_eq!(listed["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(listed["edges"].as_array().unwrap().len(), 2);
}

/// An unknown extension tool name is a dispatch error, not a panic or silent success.
#[tokio::test]
async fn dispatch_rejects_an_unknown_extension_tool() {
    use shape_scene_core::CanvasId;
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let handle = canvases.get_or_spawn(&CanvasId::from("default")).await.unwrap();
    let registry = ExtensionRegistry::with_builtins();
    let err = registry
        .dispatch(&handle, "ext_kanban_does_not_exist", &serde_json::json!({}))
        .await
        .expect_err("unknown extension tool errors");
    assert!(!err.is_empty());
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
    // Transform3x3 is `#[serde(transparent)]` over its 3x3 array, so the
    // translation column is [0][2], [1][2].
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
