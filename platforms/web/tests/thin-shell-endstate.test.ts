// Pass B end-state: every selection-edit / template / erase authoring path routes through a
// `sceneCore.*` surface, the shell runs no op-apply of its own, and the TS authoring/geometry helpers
// that the core absorbed are deleted. These are source-level structural assertions (the scene-ownership
// style): each one FAILS if the corresponding leak is reintroduced — a shell-side op-apply, an unguarded
// order-key mint, a bypassed surface, or a resurrected dead helper.
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const exists = (rel: string) => existsSync(fileURLToPath(new URL(rel, import.meta.url)));
const appSource = read("../ui/App.svelte");
const stylesSource = read("../styles.css");
const transformsSource = read("../controller/transforms.ts");
const sceneSource = read("../renderer/scene.ts");
const objectSource = read("../shared/object.ts");
const objectPrimitivesSource = read("../controller/objectPrimitives.ts");
const engineSource = read("../renderer/engine.ts");
const canvasHostSource = read("../controller/canvasHost.ts");
const interactionsSource = read("../controller/interactions.ts");
const imeHostSource = read("../ime/textEditHost.ts");

describe("Pass B end-state: shell authoring routes through the core surfaces", () => {
  it("App.svelte never runs the shell's own object op-apply", () => {
    // The shell authors ops and hands them to the runtime client; it never calls the core op-apply
    // directly. (`sceneClient.applyObjectOp` is the runtime client method, not the core's.)
    expect(appSource).not.toContain("sceneCore.applyObjectOp");
  });

  it("App.svelte mints the authoring order key only through the core (the `~` mint is fallback-gated)", () => {
    // The shell never invents the authoring order key: both helpers delegate to the core fractional
    // indexer, and the pre-load `~` mint survives ONLY behind the `if (sceneCore)` early-out.
    expect(appSource).toContain("sceneCore.nextOrderKey(scene)");
    expect(appSource).toContain("sceneCore.backOrderKey(scene)");
    const tilde = appSource.indexOf("`${maxOrder}~`");
    expect(tilde, "the `~` order-key mint must still be present (the pre-load fallback)").toBeGreaterThan(-1);
    // The mint is preceded by its `if (sceneCore) return ...` early-out — no unguarded shell minting.
    const guard = appSource.lastIndexOf("if (sceneCore) return sceneCore.nextOrderKey", tilde);
    expect(guard, "the `~` mint must sit behind the `if (sceneCore)` fallback gate").toBeGreaterThan(-1);
  });

  it("group / duplicate / reorder / nudge / select / template / erase route through sceneCore.*", () => {
    for (const surface of [
      "sceneCore.groupOps(",
      "sceneCore.ungroupOps(",
      "sceneCore.duplicateOps(",
      "sceneCore.reorderStepOps(",
      "sceneCore.moveOpsForPick(",
      "sceneCore.selectAll(",
      "sceneCore.validSelection(",
      "sceneCore.templateAnchor(",
      "sceneCore.partialEraseOps("
    ]) {
      expect(appSource, `App.svelte must route through ${surface}`).toContain(surface);
    }
  });
});

describe("Pass B end-state: the core-absorbed TS helpers are deleted", () => {
  it("controller/transforms.ts exports none of the deleted geometry/authoring helpers", () => {
    for (const dead of [
      "shiftTransform",
      "unionWorldAabb",
      "rectPathQuantized",
      "worldToObjectLocalQuantized",
      "transformsEqual",
      "transformOrigin"
    ]) {
      expect(transformsSource, `transforms.ts must not export ${dead}`).not.toContain(`export function ${dead}`);
    }
  });

  it("renderer/scene.ts exports no rect-intersect / world-rect-to-screen helpers", () => {
    for (const dead of ["rectsIntersect", "pointInRect", "worldRectToScreen"]) {
      expect(sceneSource, `scene.ts must not export ${dead}`).not.toContain(`export function ${dead}`);
    }
  });

  it("shared/object.ts has no opPrimaryTargetId (consumers read WireOp.objectId)", () => {
    expect(objectSource).not.toContain("opPrimaryTargetId");
  });
});

describe("Final-pass end-state: create-release / detach / projection / rotate-snap route through the core", () => {
  it("controller/objectPrimitives.ts holds no create/snap thresholds (core consts own them)", () => {
    // The create/snap thresholds live in scene-core (recognize::MIN_DRAG_EXTENT_PX,
    // CREATE_ANCHOR_REUSE_TOLERANCE_PX, MERGE_ENDPOINT_TOLERANCE_PX); the shell must not re-mint the literals.
    for (const threshold of [
      "MIN_DRAG_EXTENT_PX",
      "CREATE_ANCHOR_REUSE_TOLERANCE_PX",
      "MERGE_ENDPOINT_TOLERANCE_PX"
    ]) {
      expect(objectPrimitivesSource, `objectPrimitives.ts must not declare ${threshold}`).not.toContain(`${threshold} =`);
    }
  });

  it("controller/objectPrimitives.ts authors no create-release / detach geometry of its own", () => {
    // The release-resolution + detach authoring moved into the core; the shell never reimplements it here.
    for (const dead of ["resolveCreateRelease", "synthesizeReleaseAnchors", "altDetachOps"]) {
      expect(objectPrimitivesSource, `objectPrimitives.ts must not author ${dead}`).not.toContain(dead);
    }
  });

  it("App.svelte routes create-release + detach through sceneCore.* (not a shell reimplementation)", () => {
    // Release resolution and detach move-ops are core surfaces; the shell calls them, it does not reauthor them.
    expect(appSource).toContain("sceneCore.resolveCreateRelease(");
    expect(appSource).toContain("sceneCore.synthesizeCreateAnchorsBoth(");
    // The Alt-detach branch authors through the core's `detachMoveOps` (via commitBodyDrag), never a local op-build.
    expect(appSource).not.toContain("synthesizeReleaseAnchors");
    expect(appSource).not.toContain("altDetachOps");
  });

  it("renderer/scene.ts exports no screen<->world projection helpers (projection via engine.project*)", () => {
    // Screen<->world projection runs through the live core camera (engine.projectScreenToWorld /
    // projectWorldToScreen); the renderer adapter holds no TS affine of its own.
    for (const dead of ["screenToWorld", "worldToScreen"]) {
      expect(sceneSource, `scene.ts must not export ${dead}`).not.toContain(`export function ${dead}`);
    }
  });

  it("renderer/engine.ts has no shell rotate-snap matrix (rotate-snap via setCoarseRotate / core)", () => {
    // The rotate drag is snapped in-core off the coarse-rotate mode bit (engine.setCoarseRotate); the
    // shell never decomposes/rebuilds the rotate matrix.
    expect(engineSource).not.toContain("snapRotateDeltaMatrix");
  });

  it("controller/canvasHost.ts holds no shell render-projection helpers (projection via projectObjectScene)", () => {
    // The core owns the ObjectScene -> RenderObjectScene projection (and stroke de-quant) via
    // projectObjectScene; the host must not reproject scenes or strokes in TS.
    for (const dead of ["objectSceneToRenderObjectScene", "projectStroke"]) {
      expect(canvasHostSource, `canvasHost.ts must not hold ${dead}`).not.toContain(dead);
    }
  });

  it("controller/interactions.ts no longer computes the create-preview / snap-ring geometry in TS", () => {
    // The kappa-Bezier ellipse helper is deleted (no caller remains after the create-preview + ring swaps);
    // both the preview and the ring now source their geometry from the core. Reintroducing a TS path-string
    // or kappa construction for the preview/ring fails one of these.
    expect(interactionsSource, "previewEllipsePath must be deleted").not.toContain("previewEllipsePath");
    // No literal kappa constant survives for a preview/ring ellipse (the kappa math lives only in the core).
    expect(interactionsSource, "no TS kappa ellipse math for the preview/ring").not.toContain("0.5523");
    // Both transient builders route through the core's build_primitive_from_drag.
    expect(interactionsSource).toContain("core.buildPrimitiveFromDrag");
    const createPreview = interactionsSource.indexOf("export function createPreviewObject");
    expect(createPreview, "createPreviewObject must exist").toBeGreaterThan(-1);
    const createPreviewBody = interactionsSource.slice(
      createPreview,
      interactionsSource.indexOf("export function snapIndicatorObject", createPreview)
    );
    expect(createPreviewBody).toContain("core.buildPrimitiveFromDrag(kind, span");
    const snapRing = interactionsSource.indexOf("export function snapIndicatorObject");
    const snapRingBody = interactionsSource.slice(
      snapRing,
      interactionsSource.indexOf("export function buildFeedScene", snapRing)
    );
    expect(snapRingBody).toContain('core.buildPrimitiveFromDrag("ellipse"');
    // The preview builder no longer assembles a closed-rect / line path-string by hand.
    expect(createPreviewBody, "no hand-built rect path-string").not.toContain("L 0 ${q(h)} Z");
    expect(createPreviewBody, "no hand-built line path-string").not.toContain('`M 0 0 L ${q(');
  });

  it("controller/interactions.ts routes every inspector edit through the core (no shell-side op synthesis)", () => {
    // The inspector authoring path (inspectorEditOp) must own NO canvas math nor op synthesis:
    // a transform field edit routes through objectSetTransformField, a resize through objectResizeAxis,
    // and EVERY other property edit (style/text/sizing/layout/meta/clip) forwards to the core's single
    // objectInspectorEditOp — which owns the per-field op body, all defaults, and the px re-quantization.
    // Reintroducing TS matrix/angle/round math or a hand-built op body here fails one of these.
    const start = interactionsSource.indexOf("export function inspectorEditOp");
    const end = interactionsSource.indexOf("export function previewPaint");
    expect(start, "inspectorEditOp must exist").toBeGreaterThan(-1);
    expect(end, "previewPaint must follow the authoring path").toBeGreaterThan(start);
    const authoring = interactionsSource.slice(start, end);

    // Transform/resize each defer to the core surface; every other field defers to objectInspectorEditOp.
    expect(authoring).toContain("core.objectSetTransformField(");
    expect(authoring).toContain("core.objectResizeAxis(");
    expect(authoring).toContain("core.objectInspectorEditOp(");

    // No angle-unit (deg<->rad) math: the shell never knows the inspector shows degrees while the matrix
    // stores radians — that conversion is the core's (object_set_transform_field).
    expect(authoring, "no deg<->rad math in the shell").not.toContain("Math.PI");
    expect(authoring, "no /180 deg conversion").not.toContain("/ 180");
    expect(authoring, "no *180 deg conversion").not.toContain("* 180");

    // No raw matrix construction (the core composes/decomposes); no shell-side round() quantization
    // discipline (the core owns it); no hand-synthesized op body — set-style/set-text/set-sizing/
    // set-layout bodies are the core's, never assembled in the shell.
    expect(authoring, "no hand-built 3x3 matrix").not.toContain("[[");
    expect(authoring, "no shell-side quantization round()").not.toContain("Math.round");
    expect(authoring, "no hand-built set-style op body").not.toContain('"set-style"');
    expect(authoring, "no hand-built set-text op body").not.toContain('"set-text"');
    expect(authoring, "no hand-built set-sizing op body").not.toContain('"set-sizing"');
    expect(authoring, "no hand-built set-layout op body").not.toContain('"set-layout"');
    expect(authoring, "no hardcoded geometry quantum in the authoring path").not.toContain("GEOMETRY_QUANTUM_PER_PX");
  });

  it("App.svelte commitTextEdit carries no shell-side 'edit.value === current' dedup", () => {
    // An unchanged set-text degrades to a no-op in the core (identical apply + skipped empty-Batch
    // inverse), so the shell adds no dedup of its own.
    const commit = appSource.indexOf("function commitTextEdit");
    expect(commit, "commitTextEdit must exist").toBeGreaterThan(-1);
    const body = appSource.slice(commit, appSource.indexOf("const TEMPLATES", commit));
    expect(body).not.toContain("=== current");
    expect(body).not.toContain("edit.value ===");
  });

  it("child-select forwards the active-container token to the core (no shell-side scoped pick)", () => {
    // The drill-in scope is a forwarded token like setCoarseRotate: the shell mirrors the core-resolved
    // id and hands it back via setActiveContainer, and the CORE scopes the next pointer-down pick. The
    // shell must NOT narrow the hit-test itself. These FAIL if the forward is removed or the shell starts
    // picking the child off a TS parent comparison instead of forwarding the token.
    const canvasHostSrc = read("../controller/canvasHost.ts");
    // The host exposes a pure passthrough to the wasm renderer (no selection logic).
    expect(canvasHostSrc, "canvasHost must forward setActiveContainer to the renderer").toContain(
      "this.webGpuRenderer?.setActiveContainer?.("
    );
    // The drill-in branch forwards the core-resolved id; scope-exit forwards null. Both ride the host
    // passthrough — there is no shell-side hit_object_scoped reimplementation.
    expect(appSource, "drill-in must forward the active-container token").toContain("host?.setActiveContainer(id)");
    expect(appSource, "scope-exit must forward null to the core").toContain("host?.setActiveContainer(null)");
    // The scope-exit decision now lives in the core, not the shell: the shell forwards selection +
    // container to objectSelectionInScope and mirrors the verdict; it owns no scope-exit predicate.
    expect(appSource, "the shell-side scope-exit predicate must be deleted").not.toContain("function selectionLeavesScope");
    expect(appSource, "scope-exit must route through the core verdict").toContain("sceneCore.objectSelectionInScope(");
  });

  it("the UI two-tier dispatch leaks no UI decision into TS (engine/App forward + render only)", () => {
    // engine.ts forwards pointer/key to the UI runtime and relays the verdict; it computes NOTHING from
    // the dispatch result (no slider value, no hit decision, no focus predicate of its own). These FAIL
    // if a UI decision is reimplemented in TS.
    // (1) No arithmetic on a UI dispatch result — the shell never recomputes a slider value / cell index.
    expect(engineSource, "no shell-side slider math off a UI dispatch result").not.toContain("result.value");
    expect(engineSource, "no shell-side segment index off a UI dispatch result").not.toContain("result.index");
    // (2) The UI pointer routing reads ONLY `consumed`/`sceneChanged` off the dispatch (a forward verdict),
    // never a payload it would act on.
    expect(engineSource).toContain("return { consumed: result.consumed, sceneChanged: result.sceneChanged };");
    // (3) Actions are relayed OPAQUELY as EngineEvents — a verbatim forward, not a branch.
    expect(engineSource).toContain('this.onEvent({ type: "ui-action", action });');

    // App.svelte never branches the UI key forward on an event.key literal (the catalog stays the single
    // binding source); the focus arbiter ORs the core verdict (uiHasFocus), it computes no predicate itself.
    const forwardLine = appSource
      .split("\n")
      .find((line) => line.includes("host?.uiKey({ key: event.key, text: keyChar(event),"));
    expect(forwardLine, "the uiKey forward must exist").toBeDefined();
    expect(forwardLine, "the uiKey forward must not branch on a key literal").not.toMatch(/event\.key\s*===/);
    expect(appSource, "the focus arbiter ORs the core verdict, no second TS focus predicate").toContain(
      "host?.uiHasFocus() ?? false"
    );
  });

  it("App.svelte declares sceneCore as $state (the inspector $derived must track it reactively)", () => {
    // Regression guard: `sceneCore` was once a plain non-reactive `let`, so the inspectorView $derived
    // captured no reactive dep on it — it short-circuited on `!sceneCore` before reading selection/scene
    // and never recomputed once the core loaded, so the property panel never appeared. Declaring it via
    // $state makes the assignment after load re-run the $derived. This FAILS if it reverts to a plain let.
    expect(appSource, "sceneCore must be reactive ($state) so the inspector $derived recomputes on load")
      .toMatch(/let\s+sceneCore\s*=\s*\$state<SceneCore\s*\|\s*null>\(null\)/);
    // And specifically NOT a plain `let sceneCore = ...` (the broken form that captured no dep).
    expect(appSource, "sceneCore must not revert to a plain non-reactive let")
      .not.toMatch(/let\s+sceneCore\s*=\s*null\s*;/);
    expect(appSource, "sceneCore must not revert to a plain non-reactive typed let")
      .not.toMatch(/let\s+sceneCore\s*:\s*SceneCore\s*\|\s*null\s*=\s*null\s*;/);
  });
});

describe("IME host-port library stays a pure host-port (no decision, no op, no canvas state)", () => {
  it("ime/textEditHost.ts authors no op and imports no core/scene authoring", () => {
    // The library realizes the OS editing surface and hands back ONE committed string; it must construct
    // no op, run no op-apply, and import nothing from the core/scene authoring layers. FAILS if a decision
    // (an op kind, a sceneCore call, an ObjectOp import) leaks into the library.
    for (const leak of [
      '"set-text"',
      "authorOp",
      "sceneCore",
      "ObjectOp",
      "applyObjectOp",
      "../shared/object",
      "../bridge/sceneCoreWasm"
    ]) {
      expect(imeHostSource, `the IME library must not contain ${leak}`).not.toContain(leak);
    }
  });

  it("ime/textEditHost.ts computes nothing from the committed string beyond passing it back", () => {
    // The committed-string callback is a pure forward: onCommit hands the live textContent straight to the
    // consumer. The library does no string transform / dedup / parse — that is the consumer's (App authors
    // the op; the core decides). FAILS if the library starts comparing/transforming the committed value.
    expect(imeHostSource, "onCommit forwards the value verbatim").toContain("active.cb.onCommit(value)");
    expect(imeHostSource, "the library does no committed-string dedup").not.toContain("=== current");
    // No canvas/world coordinate math: the mount rect is consumed as opaque screen px (the consumer does
    // the world->screen derive). The library only writes the four position style props.
    expect(imeHostSource, "no world projection in the library").not.toContain("projectWorldToScreen");
    expect(imeHostSource, "no world projection in the library").not.toContain("projectScreenToWorld");
  });

  it("App.svelte authors the canvas set-text op (the library only hands back the string)", () => {
    // The op stays authored in App.svelte's commitTextEdit (via authorOp); the library never authors it.
    const commit = appSource.indexOf("function commitTextEdit");
    const body = appSource.slice(commit, appSource.indexOf("const TEMPLATES", commit));
    expect(body, "commitTextEdit authors the set-text op in the shell, not the library").toContain(
      'authorOp({ kind: "set-text"'
    );
  });

  it("the canvas inline edit and the ui-core TextInput edit share ONE TextEditHost instance", () => {
    // Both consumers route through the single `ensureTextEditHost()` instance — never two divergent
    // overlays. FAILS if a second TextEditHost is constructed for either consumer.
    expect(appSource, "the shared host is built once via ensureTextEditHost").toContain("function ensureTextEditHost");
    // Exactly one `new TextEditHost(` in App.svelte (inside ensureTextEditHost); both edits reuse it.
    const constructions = appSource.split("new TextEditHost(").length - 1;
    expect(constructions, "App.svelte constructs the IME host exactly once").toBe(1);
  });

  it("engine.ts relays the ui-core EditRequest instead of dropping it (the dead-wire fix)", () => {
    // The ui-core IME path was dead: uiPointer forwarded actions but discarded result.edit, so a focused
    // TextInput could never mount an editing surface. The relay must surface it as a ui-edit event. FAILS
    // if the edit is dropped again.
    expect(engineSource, "uiPointer must relay result.edit as a ui-edit event").toContain(
      'this.onEvent({ type: "ui-edit", edit: result.edit });'
    );
  });
});

describe("P4 end-state: no TS product UI — every product UI lives in the Rust ui extension", () => {
  // The six product-UI Svelte components moved into the Rust `shape_ui` extension; the shell renders only
  // the surface + OS-input forwarding and routes the UI through the model+intent seam. Each assertion FAILS
  // if a TS product-UI component is resurrected, the model/intent seam is removed, or the shell starts
  // building product-UI markup / branching UI on a key literal again.
  const deletedComponents = [
    "../ui/Toolbar.svelte",
    "../ui/SettingsModal.svelte",
    "../ui/InspectorPanel.svelte",
    "../ui/ContextMenu.svelte",
    "../ui/TemplatePopup.svelte",
    "../ui/PeerCursors.svelte"
  ];

  it("none of the six product-UI Svelte components exist (a resurrected one fails the build)", () => {
    for (const path of deletedComponents) {
      expect(exists(path), `${path} must be deleted — its UI lives in the Rust ui extension`).toBe(false);
    }
    // The surface provider + the shell entry survive (those are core shell jobs, not product UI).
    expect(exists("../ui/ShapeCanvasHost.svelte"), "the surface provider stays").toBe(true);
    expect(exists("../ui/App.svelte"), "the shell entry stays").toBe(true);
  });

  it("App.svelte imports no product-UI .svelte component except the surface provider", () => {
    // The only `*.svelte` import the shell keeps is ShapeCanvasHost (the surface). Any other component
    // import is a resurrected product UI.
    const svelteImports = appSource
      .split("\n")
      .filter((line) => /import\s+.*from\s+"\.\/[^"]+\.svelte"/.test(line));
    expect(svelteImports.length, "exactly one .svelte import (the surface provider)").toBe(1);
    expect(svelteImports[0]).toContain("./ShapeCanvasHost.svelte");
  });

  it("App.svelte routes the UI through the model+intent seam (not a status-only relay)", () => {
    // The shell FEEDS the model (host.setUiModel) and ROUTES the core-resolved intents to its existing
    // handlers via a typed intent switch. FAILS if the feed or the intent routing is removed.
    expect(appSource, "the shell feeds the UI model").toContain("host?.setUiModel(");
    expect(appSource, "the intent handler dispatches typed UiIntents").toContain("function handleUiIntent(intent: UiIntent)");
    expect(appSource, "a Command intent routes to the existing handlers").toContain('case "command":');
    expect(appSource, "an InspectorEdit intent lowers through the existing inspector authoring").toContain('case "inspectorEdit":');
    // The host no longer relays a UI action as an informational status string (the intent carries the
    // route now). FAILS if the status-only relay is reintroduced.
    expect(canvasHostSource, "no status-string-only ui-action relay").not.toContain("`ui-action ${event.action.type}");
  });

  it("App.svelte builds NO product-UI markup (every panel/menu is Rust-rendered)", () => {
    // The shell's markup is the surface wrapper + the CanvasHost mount; it composes no product-UI DOM.
    // Scope the check to the MARKUP (after `</script>`) — a command id like `open-template-library` lives
    // in the script and is legitimate. A reintroduced toolbar/inspector/settings/menu/template/diagnostics
    // class literal in the markup fails here.
    const markup = appSource.slice(appSource.indexOf("</script>"));
    for (const literal of ['class="toolbar', 'class="inspector', 'class="settings-modal', 'class="node-context-menu', 'class="template-library', 'class="diagnostics-panel']) {
      expect(markup, `App.svelte markup must build no '${literal}' panel`).not.toContain(literal);
    }
  });

  it("App.svelte never branches UI behavior on a key literal (the catalog stays the binding source)", () => {
    // The keydown arbiter forwards the key to the core and branches only on the catalog dispatch, never on
    // a key-literal BEHAVIOR read for a UI action. (Escape/Space are handled through their own non-UI
    // paths already pinned above.) The uiKey forward must not compare event.key to a literal.
    const forwardLine = appSource
      .split("\n")
      .find((line) => line.includes("host?.uiKey({ key: event.key, text: keyChar(event),"));
    expect(forwardLine, "the uiKey forward must exist").toBeDefined();
    expect(forwardLine, "the uiKey forward must not branch on a key literal").not.toMatch(/event\.key\s*===/);
  });

  it("styles.css holds none of the deleted product-UI selectors", () => {
    // The product-UI CSS is Rust-rendered now; a resurrected selector means a resurrected TS panel.
    for (const selector of [".toolbar-remote", ".inspector-panel", ".settings-modal", ".node-context-menu", ".template-library", ".diagnostics-panel"]) {
      expect(stylesSource, `styles.css must not hold ${selector}`).not.toContain(selector);
    }
  });
});
