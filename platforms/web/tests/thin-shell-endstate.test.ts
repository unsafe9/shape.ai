// Pass B end-state: every selection-edit / template / erase authoring path routes through a
// `sceneCore.*` surface, the shell runs no op-apply of its own, and the TS authoring/geometry helpers
// that the core absorbed are deleted. These are source-level structural assertions (the scene-ownership
// style): each one FAILS if the corresponding leak is reintroduced — a shell-side op-apply, an unguarded
// order-key mint, a bypassed surface, or a resurrected dead helper.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const appSource = read("../ui/App.svelte");
const transformsSource = read("../controller/transforms.ts");
const sceneSource = read("../renderer/scene.ts");
const objectSource = read("../shared/object.ts");
const objectPrimitivesSource = read("../controller/objectPrimitives.ts");
const engineSource = read("../renderer/engine.ts");
const canvasHostSource = read("../controller/canvasHost.ts");
const interactionsSource = read("../controller/interactions.ts");

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

  it("App.svelte commitTextEdit carries no shell-side 'edit.value === current' dedup", () => {
    // An unchanged set-text degrades to a no-op in the core (identical apply + skipped empty-Batch
    // inverse), so the shell adds no dedup of its own.
    const commit = appSource.indexOf("function commitTextEdit");
    expect(commit, "commitTextEdit must exist").toBeGreaterThan(-1);
    const body = appSource.slice(commit, appSource.indexOf("function cancelTextEdit", commit));
    expect(body).not.toContain("=== current");
    expect(body).not.toContain("edit.value ===");
  });
});
