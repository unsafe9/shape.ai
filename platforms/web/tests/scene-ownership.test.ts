// C1 keystone: the Rust core is the single source of truth for the scene, INCLUDING the offline /
// pre-connect path. The shell `scene` is a render-only mirror — assigned ONCE, inside the onScene mirror
// (commitClientScene) — and the shell never runs its own op-apply. These are structural assertions over
// App.svelte source: they fail if a future edit reintroduces a shell-owned scene or a second op-apply.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const appSource = readFileSync(fileURLToPath(new URL("../ui/App.svelte", import.meta.url)), "utf8");

describe("C1: scene ownership lives in the core, not the shell", () => {
  it("authorOp never runs the shell's own op-apply (sceneCore.applyObjectOp is gone from App)", () => {
    // The offline path must author through the core session, not a shell-side apply onto a shell scene.
    expect(appSource).not.toContain("sceneCore.applyObjectOp");
  });

  it("the offline path authors through the core session", () => {
    // Proves the pre-connect branch routes into the core (session.author), not a shell apply.
    expect(appSource).toContain("session.author(");
  });

  it("assigns `scene` only inside commitClientScene (the single onScene mirror)", () => {
    // Every bare `scene = ...` assignment (the mirror write) must live inside commitClientScene's body.
    // Extract the function body, then assert no `scene = ` assignment exists anywhere outside it.
    const fnStart = appSource.indexOf("function commitClientScene(");
    expect(fnStart).toBeGreaterThan(-1);
    const body = sliceBalancedBody(appSource, fnStart);
    expect(body).toContain("scene = next;");

    // A mirror REASSIGNMENT is `scene = ` at a statement boundary, excluding the `let scene =` state
    // declaration and member writes (`client.scene =`) / comparisons (`scene ==`). Find all such writes.
    const assignments = [...appSource.matchAll(/(?<!\.|let |const |var )\bscene\s*=\s*(?!=)/g)].map(
      (m) => m.index ?? -1
    );
    expect(assignments.length).toBeGreaterThan(0);
    const bodyStart = fnStart;
    const bodyEnd = fnStart + body.length;
    for (const idx of assignments) {
      expect(
        idx >= bodyStart && idx <= bodyEnd,
        `found a 'scene =' assignment at index ${idx}, outside commitClientScene [${bodyStart}, ${bodyEnd}]`
      ).toBe(true);
    }
  });

  it("clears the optimistic preview off the core's settled signal, not a transform-value compare", () => {
    // commitClientScene must consult the settledKeys signal the core hands it, never transformsEqual.
    const fnStart = appSource.indexOf("function commitClientScene(");
    const body = sliceBalancedBody(appSource, fnStart);
    expect(body).toContain("settledKeys");
    expect(body).not.toContain("transformsEqual");
  });
});

// B1: the z-order / group / duplicate / reorder / nudge authoring paths route through the core surfaces;
// the shell no longer assembles clone/group/reorder ops or mints fractional order keys in TS. Structural
// assertions over App.svelte source — they fail if a future edit reintroduces shell-side authoring.
describe("B1: selection-edit authoring routes through the core surfaces", () => {
  function fnBody(name: string): string {
    const start = appSource.indexOf(`function ${name}(`);
    expect(start, `function ${name} not found in App.svelte`).toBeGreaterThan(-1);
    return sliceBalancedBody(appSource, start);
  }

  it("duplicateSelection routes through sceneCore.duplicateOps with no shell clone authoring", () => {
    const body = fnBody("duplicateSelection");
    expect(body).toContain("sceneCore.duplicateOps(");
    // No shell-side clone assembly (the old shiftTransform / insert-object loop is gone).
    expect(body).not.toContain("shiftTransform");
    expect(body).not.toContain("insert-object");
  });

  it("groupSelection routes through sceneCore.groupOps (the core sizes + reparents)", () => {
    const body = fnBody("groupSelection");
    expect(body).toContain("sceneCore.groupOps(");
    expect(body).not.toContain("reparent");
    expect(body).not.toContain("unionWorldAabb");
  });

  it("ungroupSelection routes through sceneCore.ungroupOps", () => {
    const body = fnBody("ungroupSelection");
    expect(body).toContain("sceneCore.ungroupOps(");
    expect(body).not.toContain("reparent");
  });

  it("nudgeSelection routes through sceneCore.moveOpsForPick (the same path as a body drag)", () => {
    const body = fnBody("nudgeSelection");
    expect(body).toContain("sceneCore.moveOpsForPick(");
    // No independent per-id set-transform loop.
    expect(body).not.toContain("set-transform");
  });

  it("reorderStep routes through sceneCore.reorderStepOps (no shell neighbor swap)", () => {
    const body = fnBody("reorderStep");
    expect(body).toContain("sceneCore.reorderStepOps(");
    // No shell-authored reorder op nor a hand-rolled flat-order neighbor sort.
    expect(body).not.toContain('kind: "reorder"');
    expect(body).not.toContain("sorted");
  });

  it("the order-key helpers mint from the core; the shell `~` minting is only a pre-load fallback", () => {
    // Both helpers delegate to the core fractional indexer when loaded — the shell never invents the
    // authoring order key. The `${maxOrder}~` / ascii fallback survives only behind the `if (sceneCore)` gate.
    const next = fnBody("nextOrderKey");
    const back = fnBody("backOrderKey");
    expect(next).toContain("sceneCore.nextOrderKey(scene)");
    expect(back).toContain("sceneCore.backOrderKey(scene)");
    // The `~` mint only appears AFTER the `if (sceneCore) return ...` early-out (the fallback).
    expect(next.indexOf("if (sceneCore)")).toBeLessThan(next.indexOf("~"));
  });
});

// B2: the selection / template / erase authoring paths route through the core surfaces; the shell no
// longer hand-rolls the selection collapse rule, the template-anchor geometry, nor the object-local erase
// mapping. (The set-text dedup swap is DEFERRED — see the blocker: the core undo stack records the
// empty-Batch no-op inverse, so dropping the shell compare would push a bogus undo entry.) Structural
// assertions over App.svelte source.
describe("B2: selection / template / erase route through the core surfaces", () => {
  // Slice the function body. A brace-bearing return type or parameter type (`{ x: number; y: number }`)
  // can put a `{` in the signature, so the body is the brace group whose match reaches FARTHEST — the
  // function end — past the signature's short-lived type groups.
  function fnBody(name: string): string {
    const start = appSource.indexOf(`function ${name}(`);
    expect(start, `function ${name} not found in App.svelte`).toBeGreaterThan(-1);
    // Find the body `{`: walk from the signature, skipping every balanced `{...}` that closes on the same
    // line (a type annotation), and take the first `{` that opens a multi-line block (the body).
    let bodyOpen = -1;
    for (let i = appSource.indexOf("(", start); i < appSource.length; i++) {
      if (appSource[i] !== "{") continue;
      let depth = 0;
      let close = i;
      for (let j = i; j < appSource.length; j++) {
        if (appSource[j] === "{") depth++;
        else if (appSource[j] === "}" && --depth === 0) {
          close = j;
          break;
        }
      }
      // A type group closes on the same line (no newline inside); the body spans newlines.
      if (appSource.slice(i, close).includes("\n")) {
        bodyOpen = i;
        break;
      }
    }
    return sliceBalancedBody(appSource, bodyOpen);
  }

  it("selectAll routes through sceneCore.selectAll (no shell-side multi/object assembly)", () => {
    const body = fnBody("selectAll");
    expect(body).toContain("sceneCore.selectAll(scene)");
    expect(body).not.toContain('kind: "multi"');
  });

  it("validSelection delegates the collapse rule to sceneCore.validSelection", () => {
    const body = fnBody("validSelection");
    expect(body).toContain("sceneCore.validSelection(");
    // No hand-rolled live-id filter / collapse arithmetic in the shell.
    expect(body).not.toContain(".filter(");
    expect(body).not.toContain("live.length");
  });

  it("templateAnchor routes through sceneCore.templateAnchor passing only the viewport fallback", () => {
    const body = fnBody("templateAnchor");
    expect(body).toContain("sceneCore.templateAnchor(scene");
    expect(body).toContain("viewportCenterWorld()");
    // No shell-side right-most/top-aligned anchor scan.
    expect(body).not.toContain("transformOrigin");
    expect(body).not.toContain("maxX");
  });

  it("handleErase calls the widened sceneCore.partialEraseOps with the WORLD touch (no TS inverse-affine)", () => {
    const body = fnBody("handleErase");
    expect(body).toContain("sceneCore.partialEraseOps(scene, id, world.x, world.y");
    // The shell no longer maps the world touch into object-local quantized space.
    expect(body).not.toContain("worldToObjectLocalQuantized");
  });
});

// Return the source of a `function name(...) { ... }` starting at `fnStart`, including the braces,
// by matching balanced `{}` (ignoring the brace-counting subtleties of strings — sufficient for this
// component's body, which has no brace-bearing string literals in commitClientScene).
function sliceBalancedBody(source: string, fnStart: number): string {
  const open = source.indexOf("{", fnStart);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}") {
      depth--;
      if (depth === 0) return source.slice(fnStart, i + 1);
    }
  }
  throw new Error("unbalanced braces while slicing commitClientScene");
}
