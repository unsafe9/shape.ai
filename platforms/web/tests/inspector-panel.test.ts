// The inspector property panel is render-only: the Rust core's `objectInspectorView` decides the role,
// which controls apply, and each control's current value/mixed flag; `inspectorEditOp` lowers a panel
// edit back to an ObjectOp. These tests DRIVE THE REAL CORE (the scene-core wasm) end-to-end — every
// view is the core's, every authored op is applied through the real op-apply path — so they fail when
// the role filtering or the edit->op mapping is wrong, not merely when the code stops compiling. The
// panel's widget wiring + "Mixed" display is pinned against the .svelte source (no DOM in node).

import { beforeAll, describe, expect, it } from "vitest";

import {
  ensureSceneCore,
  loadSceneCore,
  type InspectorControlValue,
  type InspectorView,
  type SceneCore
} from "../bridge/sceneCoreWasm";
import {
  authorInspectorAction,
  authorInspectorEdit,
  inspectorActionOp,
  inspectorEditOp,
  inspectorSegmentSelected
} from "../controller/interactions";
import {
  emptyObjectScene,
  IDENTITY_TRANSFORM,
  type Layout,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type ObjectSelection
} from "../shared/object";

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

// A closed unit rect, optionally parented into `parent`. Order keys must sort distinctly.
function rect(id: string, order = "a0", parent?: string): SceneObject {
  return {
    id,
    order,
    parent: parent ?? null,
    transform: IDENTITY_TRANSFORM,
    geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z", fillRule: "evenOdd" }
  };
}

// Apply an op through the REAL core; fail loudly on a domain error so a bad fixture can't pass silently.
function apply(scene: ObjectScene, op: ObjectOp): ObjectScene {
  const result = core.applyObjectOp(scene, op);
  expect(result.errors).toEqual([]);
  return result.scene;
}

function insert(scene: ObjectScene, object: SceneObject): ObjectScene {
  return apply(scene, { kind: "insert-object", object });
}

// The dynamic inspector view the panel renders from, resolved by the real core.
function viewFor(scene: ObjectScene, selection: ObjectSelection): InspectorView {
  return core.objectInspectorView(JSON.stringify(scene), JSON.stringify(selection));
}

// Every control id present in the view, flattened across sections (the panel renders these, keyed by id).
function controlIds(view: InspectorView): Set<string> {
  return new Set(view.sections.flatMap((s) => s.controls.map((c) => c.id)));
}
function sectionNames(view: InspectorView): Set<string> {
  return new Set(view.sections.map((s) => s.section));
}
function control(view: InspectorView, id: string): InspectorControlValue {
  const found = view.sections.flatMap((s) => s.controls).find((c) => c.id === id);
  if (!found) throw new Error(`control ${id} absent from view`);
  return found;
}

// A horizontal single-lane Flow layout used to make a parent a flow container (and its children flow children).
const FLOW_LAYOUT: Layout = {
  axis: "horizontal",
  lanes: { kind: "count", value: 1 },
  spacing: 0,
  align: { main: "start", cross: "start" }
};

// ---- Panel rendering from a core-resolved view ----------------------------------------------------

describe("InspectorPanel renders the controls the core's view exposes per role", () => {
  it("a free object exposes x/y/width/height/rotation and NO flow-child sizing", () => {
    const scene = insert(emptyObjectScene(), rect("r"));
    const view = viewFor(scene, { kind: "object", id: "r" });
    const ids = controlIds(view);
    for (const id of ["x", "y", "width", "height", "rotation"]) {
      expect(ids.has(id)).toBe(true);
    }
    expect(ids.has("sizing-w")).toBe(false);
    expect(ids.has("sizing-h")).toBe(false);
    expect(ids.has("rotation-flow")).toBe(false);
    expect(view.role.placement).toBe("free");
    // x reads the identity transform's translate (0) through the core decomposition.
    expect(control(view, "x").value).toBe(0);
  });

  it("a flow child exposes sizing-w/sizing-h and NO free x/y", () => {
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));
    scene = apply(scene, { kind: "set-layout", id: "p", layout: FLOW_LAYOUT });
    const view = viewFor(scene, { kind: "object", id: "c" });
    const ids = controlIds(view);
    expect(ids.has("sizing-w")).toBe(true);
    expect(ids.has("sizing-h")).toBe(true);
    expect(ids.has("rotation-flow")).toBe(true);
    for (const id of ["x", "y", "width", "height", "rotation"]) {
      expect(ids.has(id)).toBe(false);
    }
    expect(view.role.placement).toBe("flow-child");
    // Default sizing reads Hug.
    expect((control(view, "sizing-w").value as { kind: string }).kind).toBe("hug");
  });

  it("a flow container exposes axis/lanes/spacing/align (the auto-layout inputs)", () => {
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));
    scene = apply(scene, {
      kind: "set-layout",
      id: "p",
      layout: { ...FLOW_LAYOUT, axis: "vertical", spacing: 16 }
    });
    const view = viewFor(scene, { kind: "object", id: "p" });
    const ids = controlIds(view);
    for (const id of ["layout-mode", "axis", "lanes", "spacing", "align", "clip"]) {
      expect(ids.has(id)).toBe(true);
    }
    expect(view.role.flowContainer).toBe(true);
    expect(control(view, "axis").value).toBe("vertical");
    // spacing reads honest logical px: 16 quantized units / Q(8) = 2 px (and carries that Q as unitScale).
    expect(control(view, "spacing").value).toBe(2);
    expect(control(view, "spacing").unitScale).toBe(8);
  });

  it("a text object exposes a Text section with font/size/weight/align/color", () => {
    const obj = rect("t");
    obj.text = {
      runs: [{ text: "hi", font: "Mono", size: 24, bold: true, italic: false, color: "#112233" }],
      align: "center",
      valign: "top"
    };
    const scene = insert(emptyObjectScene(), obj);
    const view = viewFor(scene, { kind: "object", id: "t" });
    expect(sectionNames(view).has("text")).toBe(true);
    const ids = controlIds(view);
    for (const id of ["font", "font-size", "font-weight", "text-align", "text-color"]) {
      expect(ids.has(id)).toBe(true);
    }
    // font-size reads honest logical px: 24 quantized units / Q(8) = 3 px.
    expect(control(view, "font-size").value).toBe(3);
    expect(control(view, "font-size").unitScale).toBe(8);

    // A non-text object hides the Text section entirely.
    const plain = insert(emptyObjectScene(), rect("r"));
    expect(sectionNames(viewFor(plain, { kind: "object", id: "r" })).has("text")).toBe(false);
  });

  it("multi-select marks diverging values mixed (value null) and keeps shared values", () => {
    const a = rect("a");
    a.fill = { paint: { kind: "solid", color: "#ff0000" }, opacity: 1 };
    const b = rect("b", "a1");
    b.fill = { paint: { kind: "solid", color: "#00ff00" }, opacity: 1 };
    let scene = insert(emptyObjectScene(), a);
    scene = insert(scene, b);
    const view = viewFor(scene, { kind: "multi", ids: ["a", "b"] });
    const fill = control(view, "fill");
    expect(fill.mixed).toBe(true);
    expect(fill.value).toBeNull();
    // Both visible (default) => not mixed.
    const visible = control(view, "visible");
    expect(visible.mixed).toBe(false);
    expect(visible.value).toBe(true);
  });
});

// ---- Segment active-state (the panel highlights the option matching the core's value) -------------

describe("inspectorSegmentSelected marks the option matching the control's current value", () => {
  it("font-weight reads the bold BOOLEAN the core emits: a bold object highlights Bold, not Regular", () => {
    const obj = rect("t");
    obj.text = {
      runs: [{ text: "hi", font: "Mono", size: 24, bold: true, italic: false, color: "#112233" }],
      align: "center",
      valign: "top"
    };
    const scene = insert(emptyObjectScene(), obj);
    const weight = control(viewFor(scene, { kind: "object", id: "t" }), "font-weight");
    // The core hands the weight down as a boolean (`true` for bold), NOT the "Bold"/"Regular" token.
    expect(weight.value).toBe(true);
    // So the panel's active-state decision must map the boolean onto the Bold/Regular segment.
    expect(inspectorSegmentSelected(weight.value, "font-weight", "Bold")).toBe(true);
    expect(inspectorSegmentSelected(weight.value, "font-weight", "Regular")).toBe(false);
  });

  it("font-weight highlights Regular for a non-bold object", () => {
    const obj = rect("t");
    obj.text = {
      runs: [{ text: "hi", font: "Mono", size: 24, bold: false, italic: false }],
      align: "start",
      valign: "top"
    };
    const scene = insert(emptyObjectScene(), obj);
    const weight = control(viewFor(scene, { kind: "object", id: "t" }), "font-weight");
    expect(weight.value).toBe(false);
    expect(inspectorSegmentSelected(weight.value, "font-weight", "Regular")).toBe(true);
    expect(inspectorSegmentSelected(weight.value, "font-weight", "Bold")).toBe(false);
  });

  it("a sizing segment matches on the AxisSizing tag, a plain segment on the lower-cased token", () => {
    expect(inspectorSegmentSelected({ kind: "fill" }, "sizing-w", "Fill")).toBe(true);
    expect(inspectorSegmentSelected({ kind: "fill" }, "sizing-w", "Hug")).toBe(false);
    expect(inspectorSegmentSelected("vertical", "axis", "Vertical")).toBe(true);
    expect(inspectorSegmentSelected("vertical", "axis", "Horizontal")).toBe(false);
  });

  it("clicking 'Flow' on an already-Flow container is the active option => the panel guard skips onEdit (Fix 1)", () => {
    // The panel's segment onclick early-returns when `segmentSelected(control, option)` is already true.
    // For a flow container the layout-mode value is "flow", so the "Flow" option IS the active one — the
    // guard short-circuits and authors NOTHING, preserving the customized layout. (The destructive reset
    // path was: fire onEdit("flow") on every click. This asserts the predicate the guard reads is true.)
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));
    scene = apply(scene, { kind: "set-layout", id: "p", layout: { ...FLOW_LAYOUT, axis: "vertical", spacing: 128 } });
    const mode = control(viewFor(scene, { kind: "object", id: "p" }), "layout-mode");
    expect(mode.value).toBe("flow");
    // The active option => panel guard returns before onEdit (no op authored).
    expect(inspectorSegmentSelected(mode.value, "layout-mode", "Flow")).toBe(true);
    // The inactive option => panel fires onEdit (a real Flow->Free change authors set-layout null).
    expect(inspectorSegmentSelected(mode.value, "layout-mode", "Free")).toBe(false);
  });
});

// The inspector panel is now rendered by the Rust `shape_ui` extension (crates/ui/src/inspector.rs +
// composites.rs), not a Svelte component — so the per-widget-kind wiring + Mixed display are pinned by the
// crates/ui Rust tests (run via the renderer-wgpu host gate), not against a .svelte source here. The
// thin-shell endstate guard (tests/thin-shell-endstate.test.ts) asserts InspectorPanel.svelte no longer
// exists. What stays shell-side and is verified above: the core's role-filtered VIEW (objectInspectorView)
// and the intent->op authoring path (inspectorEditOp/inspectorSegmentSelected), both driven through the
// real core. Nothing in this file reads a .svelte source any more.

// ---- Edit -> ObjectOp mapping (inspectorEditOp), verified by re-applying through the real core -----

describe("inspectorEditOp lowers a panel edit to the correct ObjectOp", () => {
  // Build a control value the way the core view hands it down (id/opKind/unitScale drive the mapping;
  // unitScale defaults to 1, matching every non-quantized control).
  function ctrl(id: string, opKind?: string, unitScale = 1): InspectorControlValue {
    return { id, label: id, widget: { kind: "text" }, value: null, mixed: false, opKind, unitScale };
  }

  it("spacing px edit => set-layout with the px RE-QUANTIZED (px * unitScale), rest of layout preserved", () => {
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));
    scene = apply(scene, {
      kind: "set-layout",
      id: "p",
      layout: { ...FLOW_LAYOUT, axis: "vertical", spacing: 4 }
    });
    // The user types 24 px; the catalog's unitScale (Q=8) re-quantizes it to 192 stored units.
    const op = inspectorEditOp(core, scene, ctrl("spacing", "set-layout", 8), "p", 24);
    expect(op).toEqual({
      kind: "set-layout",
      id: "p",
      // axis/lanes/align unchanged, only spacing patched (24 px => 192 quantized).
      layout: { axis: "vertical", lanes: { kind: "count", value: 1 }, spacing: 192, align: { main: "start", cross: "start" } }
    });
    // Round-trips through the real core: the view re-reads spacing 24 px.
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "p" }), "spacing").value).toBe(24);
  });

  it("visible toggle => set-meta with the hidden flag INVERTED", () => {
    const scene = insert(emptyObjectScene(), rect("r"));
    // Unchecking Visible (value=false) stores hidden=true.
    expect(inspectorEditOp(core, scene, ctrl("visible", "set-meta"), "r", false)).toEqual({
      kind: "set-meta",
      id: "r",
      hidden: true
    });
    // Checking Visible (value=true) stores hidden=false.
    expect(inspectorEditOp(core, scene, ctrl("visible", "set-meta"), "r", true)).toEqual({
      kind: "set-meta",
      id: "r",
      hidden: false
    });
  });

  it("sizing-w segment => set-sizing patching ONLY the w axis", () => {
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));
    scene = apply(scene, { kind: "set-layout", id: "p", layout: FLOW_LAYOUT });
    const op = inspectorEditOp(core, scene, ctrl("sizing-w", "set-sizing"), "c", { kind: "fill" });
    expect(op).toEqual({
      kind: "set-sizing",
      id: "c",
      sizing: { w: { kind: "fill" }, h: { kind: "hug" } }
    });
    const after = apply(scene, op!);
    expect((control(viewFor(after, { kind: "object", id: "c" }), "sizing-w").value as { kind: string }).kind).toBe("fill");
  });

  it("a Fixed sizing reads its value in px and a px edit RE-QUANTIZES through the core (Fix 2)", () => {
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));
    scene = apply(scene, { kind: "set-layout", id: "p", layout: FLOW_LAYOUT });
    // A Fixed sizing stored at 120 quantized units reads back as 120/Q(8) = 15 px, and carries Q as unitScale.
    scene = apply(scene, { kind: "set-sizing", id: "c", sizing: { w: { kind: "fixed", value: 120 }, h: { kind: "hug" } } });
    const sizingW = control(viewFor(scene, { kind: "object", id: "c" }), "sizing-w");
    expect(sizingW.value).toEqual({ kind: "fixed", value: 15 });
    expect(sizingW.unitScale).toBe(8);

    // The companion px input hands a BARE NUMBER (not an AxisSizing tag); the generic px-edit branch
    // re-quantizes it (px * unitScale, rounded in-core) and sizingEditOp wraps it as the Fixed value.
    const op = inspectorEditOp(core, scene, ctrl("sizing-w", "set-sizing", 8), "c", 32);
    expect(op).toEqual({
      kind: "set-sizing",
      id: "c",
      sizing: { w: { kind: "fixed", value: 256 }, h: { kind: "hug" } } // 32 px * 8 = 256 quantized
    });
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "c" }), "sizing-w").value).toEqual({ kind: "fixed", value: 32 });
  });

  it("layout-mode: Free => set-layout null, Flow => set-layout with a default Layout", () => {
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));

    const toFlow = inspectorEditOp(core, scene, ctrl("layout-mode", "set-layout"), "p", "flow");
    expect(toFlow).toEqual({
      kind: "set-layout",
      id: "p",
      layout: { axis: "horizontal", lanes: { kind: "count", value: 1 }, spacing: 0, align: { main: "start", cross: "start" } }
    });

    // After turning Flow on, switching to Free clears the layout. The core authors set-layout with a
    // None layout, which serializes with the layout field OMITTED (absent == null clears it the same).
    const flowed = apply(scene, toFlow!);
    const toFree = inspectorEditOp(core, flowed, ctrl("layout-mode", "set-layout"), "p", "free");
    expect(toFree).toEqual({ kind: "set-layout", id: "p" });
    // Round-trips through the real core: the container reads Free again.
    expect(control(viewFor(apply(flowed, toFree!), { kind: "object", id: "p" }), "layout-mode").value).toBe("free");
  });

  it("re-clicking Flow on an ALREADY-customized flow container is a no-op (preserves its layout)", () => {
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));
    // A flow container customized away from the default: vertical, Fill lanes, spacing, center/stretch.
    const customized: Layout = {
      axis: "vertical",
      lanes: { kind: "fill" },
      spacing: 128,
      align: { main: "center", cross: "stretch" }
    };
    scene = apply(scene, { kind: "set-layout", id: "p", layout: customized });
    // The "Flow" segment stays rendered + clickable while active; re-clicking it must NOT reset the
    // layout to the default (which would silently discard axis/lanes/spacing/align as a destructive undo).
    const reclick = inspectorEditOp(core, scene, ctrl("layout-mode", "set-layout"), "p", "flow");
    expect(reclick).toBeNull();
    // A FREE container (no layout) still synthesizes the default when first switched to Flow.
    let free = insert(emptyObjectScene(), rect("q"));
    free = insert(free, rect("d", "a1", "q"));
    const toFlow = inspectorEditOp(core, free, ctrl("layout-mode", "set-layout"), "q", "flow");
    expect(toFlow).toEqual({
      kind: "set-layout",
      id: "q",
      layout: { axis: "horizontal", lanes: { kind: "count", value: 1 }, spacing: 0, align: { main: "start", cross: "start" } }
    });
  });

  it("a transform edit routes through the core set-transform-field surface (NO matrix math in the shell)", () => {
    const scene = insert(emptyObjectScene(), rect("r"));
    let fieldCalls = 0;
    let seenField = "";
    // Spy on the in-core patch to prove the shell does NO matrix math itself — it hands the field +
    // value to the core and authors the returned matrix.
    const spied: Pick<SceneCore, "objectSetTransformField"> = {
      objectSetTransformField: (json, field, value) => {
        fieldCalls++;
        seenField = field;
        return core.objectSetTransformField(json, field, value);
      }
    };
    const op = inspectorEditOp(spied as SceneCore, scene, ctrl("x", "set-transform"), "r", 120);
    expect(op?.kind).toBe("set-transform");
    expect(fieldCalls).toBe(1);
    expect(seenField).toBe("x");
    // The authored transform round-trips through the real core: x reads back 120.
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "r" }), "x").value).toBe(120);
  });

  it("name text => set-meta with name set (empty string clears it)", () => {
    const scene = insert(emptyObjectScene(), rect("r"));
    expect(inspectorEditOp(core, scene, ctrl("name", "set-meta"), "r", "Hero")).toEqual({
      kind: "set-meta",
      id: "r",
      name: { action: "set", value: "Hero" }
    });
    expect(inspectorEditOp(core, scene, ctrl("name", "set-meta"), "r", "")).toEqual({
      kind: "set-meta",
      id: "r",
      name: { action: "clear" }
    });
  });

  it("rotation transform edit authors set-transform; the degrees input round-trips through the core", () => {
    const scene = insert(emptyObjectScene(), rect("r"));
    const op = inspectorEditOp(core, scene, ctrl("rotation", "set-transform"), "r", 90);
    expect(op?.kind).toBe("set-transform");
    // 90deg in => the core reads 90deg back out (the view exposes rotation in degrees).
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "r" }), "rotation").value).toBeCloseTo(90, 6);
  });

  it("a width edit routes through the core resize helper (NO scale/geometry math in the shell)", () => {
    const scene = insert(emptyObjectScene(), rect("r"));
    let resizeCalls = 0;
    let resizeAxis = "";
    // Spy on the in-core resize to prove the shell defers the absolute-px geometry math to the core.
    const spied: Pick<SceneCore, "objectResizeAxis"> = {
      objectResizeAxis: (t, g, axis, target) => {
        resizeCalls++;
        resizeAxis = axis;
        return core.objectResizeAxis(t, g, axis, target);
      }
    };
    // rect() is 10 px wide at identity; setting width to 25 px must read back 25 px through the core.
    const op = inspectorEditOp(spied as SceneCore, scene, ctrl("width", "set-transform"), "r", 25);
    expect(op?.kind).toBe("set-transform");
    expect(resizeCalls).toBe(1);
    expect(resizeAxis).toBe("x");
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "r" }), "width").value).toBeCloseTo(25, 6);

    // A height edit targets the y axis and reads back the absolute px.
    let heightAxis = "";
    const spiedH: Pick<SceneCore, "objectResizeAxis"> = {
      objectResizeAxis: (t, g, axis, target) => {
        heightAxis = axis;
        return core.objectResizeAxis(t, g, axis, target);
      }
    };
    const opH = inspectorEditOp(spiedH as SceneCore, scene, ctrl("height", "set-transform"), "r", 12.5);
    expect(heightAxis).toBe("y");
    const afterH = apply(scene, opH!);
    expect(control(viewFor(afterH, { kind: "object", id: "r" }), "height").value).toBeCloseTo(12.5, 6);
  });

  // A flow container fixture so the layout-field edits have a layout to patch and re-read.
  function flowContainer(): { scene: ObjectScene } {
    let scene = insert(emptyObjectScene(), rect("p"));
    scene = insert(scene, rect("c", "a1", "p"));
    scene = apply(scene, { kind: "set-layout", id: "p", layout: FLOW_LAYOUT });
    return { scene };
  }

  it("axis segment => set-layout patching ONLY axis (rest preserved), round-tripping through the core", () => {
    const { scene } = flowContainer();
    const op = inspectorEditOp(core, scene, ctrl("axis", "set-layout"), "p", "vertical");
    expect(op).toEqual({
      kind: "set-layout",
      id: "p",
      layout: { axis: "vertical", lanes: { kind: "count", value: 1 }, spacing: 0, align: { main: "start", cross: "start" } }
    });
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "p" }), "axis").value).toBe("vertical");
  });

  it("lanes => set-layout patching ONLY lanes; the Fill tag round-trips through the core", () => {
    const { scene } = flowContainer();
    const op = inspectorEditOp(core, scene, ctrl("lanes", "set-layout"), "p", { kind: "fill" });
    expect(op).toEqual({
      kind: "set-layout",
      id: "p",
      layout: { axis: "horizontal", lanes: { kind: "fill" }, spacing: 0, align: { main: "start", cross: "start" } }
    });
    const after = apply(scene, op!);
    expect((control(viewFor(after, { kind: "object", id: "p" }), "lanes").value as { kind: string }).kind).toBe("fill");
  });

  it("align => set-layout patching ONLY align; spaceBetween/stretch round-trip through the core", () => {
    const { scene } = flowContainer();
    const op = inspectorEditOp(core, scene, ctrl("align", "set-layout"), "p", { main: "spaceBetween", cross: "stretch" });
    expect(op).toEqual({
      kind: "set-layout",
      id: "p",
      layout: { axis: "horizontal", lanes: { kind: "count", value: 1 }, spacing: 0, align: { main: "spaceBetween", cross: "stretch" } }
    });
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "p" }), "align").value).toEqual({ main: "spaceBetween", cross: "stretch" });
  });

  it("clip toggle => set-clip; round-trips through the core", () => {
    const { scene } = flowContainer();
    const op = inspectorEditOp(core, scene, ctrl("clip", "set-clip"), "p", true);
    expect(op).toEqual({ kind: "set-clip", id: "p", clip: true });
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "p" }), "clip").value).toBe(true);
  });

  it("fill paint => set-style keeping opacity; the solid color round-trips through the core", () => {
    const r = rect("r");
    r.fill = { paint: { kind: "solid", color: "#ff0000" }, opacity: 0.5 };
    const scene = insert(emptyObjectScene(), r);
    const op = inspectorEditOp(core, scene, ctrl("fill", "set-style"), "r", { kind: "solid", color: "#00ff00" });
    expect(op).toEqual({
      kind: "set-style",
      id: "r",
      // The object's current opacity (0.5) is kept; only the paint swaps.
      fill: { action: "set", value: { paint: { kind: "solid", color: "#00ff00" }, opacity: 0.5 } }
    });
    const after = apply(scene, op!);
    expect((control(viewFor(after, { kind: "object", id: "r" }), "fill").value as { color: string }).color).toBe("#00ff00");
  });

  it("stroke-width px edit => set-style with the px RE-QUANTIZED; a borderless object gains the CORE default stroke, width round-trips px", () => {
    const scene = insert(emptyObjectScene(), rect("r"));
    // The user types 6 px; the core re-quantizes it via unitScale (Q=8) to 48 stored units and gives a
    // borderless object the SAME default stroke build_set_style_op gives a borderless open path
    // (#5b6472, the core's default-stroke convention) — the shell invents no color/attributes.
    const op = inspectorEditOp(core, scene, ctrl("stroke-width", "set-style", 8), "r", 6);
    expect(op).toEqual({
      kind: "set-style",
      id: "r",
      // dash is omitted on the wire (empty Vec skips serializing), so the op carries no dash field.
      stroke: { action: "set", value: { paint: { kind: "solid", color: "#5b6472" }, width: 48, opacity: 1, cap: "butt", join: "miter" } }
    });
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "r" }), "stroke-width").value).toBe(6);
  });

  // A text object so the set-text field edits have a run to patch and re-read.
  function textRect(): ObjectScene {
    const obj = rect("t");
    obj.text = {
      runs: [{ text: "hi", font: "Mono", size: 24, bold: false, italic: false, color: "#112233" }],
      align: "start",
      valign: "top"
    };
    return insert(emptyObjectScene(), obj);
  }

  it("font segment => set-text patching runs[0].font; round-trips through the core", () => {
    const scene = textRect();
    const op = inspectorEditOp(core, scene, ctrl("font", "set-text"), "t", "serif");
    expect(op?.kind).toBe("set-text");
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "t" }), "font").value).toBe("serif");
  });

  it("font-size px edit => set-text patching runs[0].size with the px RE-QUANTIZED; round-trips px", () => {
    const scene = textRect();
    // The user types 40 px; unitScale (Q=8) re-quantizes it to 320 stored units.
    const op = inspectorEditOp(core, scene, ctrl("font-size", "set-text", 8), "t", 40);
    expect(op).toEqual({ kind: "set-text", id: "t", text: { runs: [{ text: "hi", font: "Mono", size: 320, bold: false, italic: false, color: "#112233" }], align: "start", valign: "top" } });
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "t" }), "font-size").value).toBe(40);
  });

  it("font-weight segment => set-text setting runs[0].bold; 'bold' round-trips as true, 'regular' as false", () => {
    const scene = textRect();
    const toBold = inspectorEditOp(core, scene, ctrl("font-weight", "set-text"), "t", "bold");
    const bolded = apply(scene, toBold!);
    expect(control(viewFor(bolded, { kind: "object", id: "t" }), "font-weight").value).toBe(true);
    const toRegular = inspectorEditOp(core, bolded, ctrl("font-weight", "set-text"), "t", "regular");
    const regularized = apply(bolded, toRegular!);
    expect(control(viewFor(regularized, { kind: "object", id: "t" }), "font-weight").value).toBe(false);
  });

  it("text-align segment => set-text patching Text.align; round-trips through the core", () => {
    const scene = textRect();
    const op = inspectorEditOp(core, scene, ctrl("text-align", "set-text"), "t", "center");
    expect(op?.kind).toBe("set-text");
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "t" }), "text-align").value).toBe("center");
  });

  it("text-color paint => set-text patching runs[0].color; the solid color round-trips through the core", () => {
    const scene = textRect();
    const op = inspectorEditOp(core, scene, ctrl("text-color", "set-text"), "t", { kind: "solid", color: "#abcdef" });
    expect(op?.kind).toBe("set-text");
    const after = apply(scene, op!);
    expect(control(viewFor(after, { kind: "object", id: "t" }), "text-color").value).toBe("#abcdef");
  });

  it("a stale id (gone from the scene) maps to no op", () => {
    const scene = insert(emptyObjectScene(), rect("r"));
    expect(inspectorEditOp(core, scene, ctrl("visible", "set-meta"), "ghost", true)).toBeNull();
  });
});

describe("inspectorActionOp lowers the canonicalize button", () => {
  function buttonCtrl(opKind?: string): InspectorControlValue {
    return { id: "canonicalize", label: "Straighten", widget: { kind: "button" }, value: null, mixed: false, opKind, unitScale: 1 };
  }

  it("canonicalize button => canonicalize op for the selected id", () => {
    expect(inspectorActionOp(buttonCtrl("canonicalize"), "r")).toEqual({ kind: "canonicalize", id: "r" });
  });

  it("a non-canonicalize button maps to no op", () => {
    expect(inspectorActionOp(buttonCtrl("set-meta"), "r")).toBeNull();
    expect(inspectorActionOp(buttonCtrl(undefined), "r")).toBeNull();
  });
});

// ---- Multi-select edit batches one op per id (one undo unit) --------------------------------------

describe("a multi-select edit produces one Batch (the shell wraps per-id ops)", () => {
  // App.svelte.onInspectorEdit / onInspectorAction call these exact helpers over the selection ids
  // (currentSelectionIds()), so driving them here pins the Batch-wrapping to the shipped code path.
  function idsOf(selection: ObjectSelection): string[] {
    return selection.kind === "object" ? [selection.id] : selection.kind === "multi" ? selection.ids : [];
  }

  it("two objects => a single Batch of two set-meta ops, applied atomically by the real core", () => {
    let scene = insert(emptyObjectScene(), rect("a"));
    scene = insert(scene, rect("b", "a1"));
    const c: InspectorControlValue = { id: "locked", label: "Locked", widget: { kind: "toggle" }, value: null, mixed: false, opKind: "set-meta", unitScale: 1 };
    const op = authorInspectorEdit(core, scene, c, idsOf({ kind: "multi", ids: ["a", "b"] }), true);
    expect(op).toEqual({
      kind: "batch",
      ops: [
        { kind: "set-meta", id: "a", locked: true },
        { kind: "set-meta", id: "b", locked: true }
      ]
    });
    // The real core applies the batch as one unit; both objects flip locked, one inverse for undo.
    const result = core.applyObjectOp(scene, op!);
    expect(result.errors).toEqual([]);
    expect(result.scene.objects.every((o) => o.locked === true)).toBe(true);
    expect(result.inverse).not.toBeNull();
  });

  it("a single-object selection authors a bare op (no Batch wrapper)", () => {
    const scene = insert(emptyObjectScene(), rect("a"));
    const c: InspectorControlValue = { id: "locked", label: "Locked", widget: { kind: "toggle" }, value: null, mixed: false, opKind: "set-meta", unitScale: 1 };
    const op = authorInspectorEdit(core, scene, c, idsOf({ kind: "object", id: "a" }), true);
    expect(op).toEqual({ kind: "set-meta", id: "a", locked: true });
  });

  it("a stale id in the selection is dropped, not wrapped into the Batch", () => {
    const scene = insert(emptyObjectScene(), rect("a"));
    const c: InspectorControlValue = { id: "locked", label: "Locked", widget: { kind: "toggle" }, value: null, mixed: false, opKind: "set-meta", unitScale: 1 };
    // "ghost" maps to null (gone from the scene); only the live id authors, and a single live id is bare.
    expect(authorInspectorEdit(core, scene, c, ["a", "ghost"], true)).toEqual({ kind: "set-meta", id: "a", locked: true });
    // Every id stale => no op at all.
    expect(authorInspectorEdit(core, scene, c, ["ghost"], true)).toBeNull();
  });

  it("a multi-select canonicalize button => a single Batch of per-id canonicalize ops", () => {
    const c: InspectorControlValue = { id: "canonicalize", label: "Straighten", widget: { kind: "button" }, value: null, mixed: false, opKind: "canonicalize", unitScale: 1 };
    expect(authorInspectorAction(c, ["a", "b"])).toEqual({
      kind: "batch",
      ops: [
        { kind: "canonicalize", id: "a" },
        { kind: "canonicalize", id: "b" }
      ]
    });
    // One selected id stays bare.
    expect(authorInspectorAction(c, ["a"])).toEqual({ kind: "canonicalize", id: "a" });
  });
});
