import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import { textOverlayScreenRect, type DragSpan } from "../controller/objectPrimitives";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { GEOMETRY_QUANTUM_PER_PX, IDENTITY_TRANSFORM, type Object as SceneObject } from "../shared/object";

const appSource = readFileSync(fileURLToPath(new URL("../ui/App.svelte", import.meta.url)), "utf8");

const Q = GEOMETRY_QUANTUM_PER_PX;

let core: SceneCore;
beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

describe("text primitive (borderless, style-less)", () => {
  it("is a borderless rect: no stroke, no fill, no default text", () => {
    const object = core.buildPrimitive("text", { x: 0, y: 0 }, "text-1", "a0");
    expect(object.stroke).toBeUndefined();
    expect(object.fill).toBeUndefined();
    expect(object.text).toBeUndefined();
    // Still a rect geometry the inline editor can size to.
    expect(object.geometry.d).toBe(`M 0 0 L ${180 * Q} 0 L ${180 * Q} ${80 * Q} L 0 ${80 * Q} Z`);
  });

  it("rectangle still carries its border + fill (text-only stripping)", () => {
    const object = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-1", "a0");
    expect(object.stroke).toBeDefined();
    expect(object.fill).toBeDefined();
  });
});

describe("textOverlayScreenRect (overlay placement)", () => {
  // A camera-equivalent projector mimicking the core's world→screen affine; the shell function no longer
  // owns this math, so the test injects it explicitly over the world AABB the real core returns.
  const project = (cam: { x: number; y: number; zoom: number }) => (world: { x: number; y: number }) => ({
    x: world.x * cam.zoom + cam.x,
    y: world.y * cam.zoom + cam.y
  });

  it("projects the core world bbox to screen (translation + zoom)", () => {
    // 180x80 text rect anchored so its top-left lands at world (100, 200).
    const object = core.buildPrimitive("text", { x: 100 + 90, y: 200 + 40 }, "text-1", "a0");
    const rect = textOverlayScreenRect(core.objectWorldAabb(object), project({ x: 50, y: 30, zoom: 2 }));
    expect(rect).not.toBeNull();
    // screen = world * zoom + cameraOffset.
    expect(rect?.x).toBeCloseTo(100 * 2 + 50);
    expect(rect?.y).toBeCloseTo(200 * 2 + 30);
    expect(rect?.width).toBeCloseTo(180 * 2);
    expect(rect?.height).toBeCloseTo(80 * 2);
  });

  it("tracks the object's transform (a dragged text rect's top-left)", () => {
    const span: DragSpan = { start: { x: 300, y: 400 }, end: { x: 100, y: 250 } };
    const object = core.buildPrimitiveFromDrag("text", span, "text-2", "a0");
    const rect = textOverlayScreenRect(core.objectWorldAabb(object), project({ x: 0, y: 0, zoom: 1 }));
    // Normalized bbox top-left is (100, 250); identity camera => same screen coords.
    expect(rect?.x).toBeCloseTo(100);
    expect(rect?.y).toBeCloseTo(250);
    expect(rect?.width).toBeCloseTo(200);
    expect(rect?.height).toBeCloseTo(150);
  });

  it("returns null when the object has no world bbox", () => {
    const empty = core.objectWorldAabb({ id: "x", order: "a0", transform: IDENTITY_TRANSFORM, geometry: { d: "", fillRule: "nonZero" } });
    expect(empty).toBeNull();
    expect(textOverlayScreenRect(empty, project({ x: 0, y: 0, zoom: 1 }))).toBeNull();
  });

  it("returns null when the projector can't project (renderer not live)", () => {
    const object = core.buildPrimitive("text", { x: 90, y: 40 }, "text-3", "a0");
    expect(textOverlayScreenRect(core.objectWorldAabb(object), () => null)).toBeNull();
  });
});

describe("text tool auto-enters inline edit (connected/async path)", () => {
  // The injected world->screen projector mirroring the core affine, same form as the block above.
  const project = (world: { x: number; y: number }) => ({ x: world.x, y: world.y });

  it("insertPrimitive's text branch holds the pending insert and auto-enters edit on the inserted id", () => {
    // Pin the shipping auto-enter (App.svelte): insertPrimitive must, in its text branch, both hold the
    // built object in pendingTextInsert (so the overlay mounts this tick) and enter inline edit on that
    // object's id. Deleting either gated line — the (e) fix — must fail this assertion.
    const insert = appSource.indexOf("function insertPrimitive");
    const body = appSource.slice(insert, appSource.indexOf("function armCreate", insert));
    expect(body, "insertPrimitive must hold the pending text insert").toContain(
      'if (kind === "text") pendingTextInsert = object;'
    );
    expect(body, "insertPrimitive must auto-enter inline edit on the inserted text id").toContain(
      'if (kind === "text") enterTextEdit(object.id);'
    );
  });

  it("the overlay mounts BEFORE the scene lands: pendingTextInsert backs textEditObject/textEditRect", () => {
    // Connected insert window: the object is built + held in pendingTextInsert but the canonical scene
    // has not reflected it yet (scene.objects empty). The fallback must yield a non-null rect so the
    // contenteditable mounts + focuses this tick. On the pre-fix derive (scene.objects only) this is null.
    const pending = core.buildPrimitive("text", { x: 90, y: 40 }, "text-pending", "a0");
    const textEdit = { id: pending.id, value: "" };

    const textEditObjectFrom = (objects: SceneObject[]): SceneObject | null =>
      objects.find((o) => o.id === textEdit.id) ??
      (pending.id === textEdit.id ? pending : null);

    const emptyScene: SceneObject[] = [];
    const rectPreScene = textOverlayScreenRect(
      core.objectWorldAabb(textEditObjectFrom(emptyScene)!),
      project
    );
    expect(rectPreScene).not.toBeNull();

    // Once the canonical scene carries the same id, the scene object wins (rect derives from scene, not
    // the stale pending object). Shift the canonical copy and confirm the rect tracks it.
    const canonical = core.buildPrimitive("text", { x: 290, y: 240 }, "text-pending", "a0");
    const resolved = textEditObjectFrom([canonical]);
    expect(resolved).toBe(canonical);
    const rectWithScene = textOverlayScreenRect(core.objectWorldAabb(resolved!), project);
    const rectCanonical = textOverlayScreenRect(core.objectWorldAabb(canonical), project);
    expect(rectWithScene).toEqual(rectCanonical);
    expect(rectWithScene).not.toEqual(rectPreScene);
  });

  it("App.svelte textEditObject falls back to pendingTextInsert until the canonical scene lands", () => {
    // Pin the fix in the shipping source: the derive must consult pendingTextInsert, not scene.objects alone.
    expect(appSource, "textEditObject must fall back to the pending insert").toContain(
      "pendingTextInsert?.id === textEdit.id ? pendingTextInsert : null"
    );
  });

  it("commitTextEdit clears BOTH textEdit and pendingTextInsert (no stale shadow of a later scene)", () => {
    // Lift the commitTextEdit body: it must null both transients so a stale pending object can't shadow a
    // later canonical scene. On the pre-fix body (only textEdit nulled) pendingTextInsert lingers.
    let textEdit: { id: string; value: string } | null = { id: "text-c", value: "hi" };
    let pendingTextInsert: SceneObject | null = core.buildPrimitive("text", { x: 0, y: 0 }, "text-c", "a0");
    const authored: unknown[] = [];

    const edit = textEdit;
    textEdit = null;
    pendingTextInsert = null;
    if (edit) authored.push({ kind: "set-text", id: edit.id, text: { runs: [{ text: edit.value }] } });

    expect(textEdit).toBeNull();
    expect(pendingTextInsert).toBeNull();
    expect(authored).toEqual([{ kind: "set-text", id: "text-c", text: { runs: [{ text: "hi" }] } }]);

    // Guard the shipping source too: the commit body nulls pendingTextInsert alongside textEdit.
    const commit = appSource.indexOf("function commitTextEdit");
    const body = appSource.slice(commit, appSource.indexOf("const TEMPLATES", commit));
    expect(body, "commitTextEdit must clear pendingTextInsert").toContain("pendingTextInsert = null");
  });
});
