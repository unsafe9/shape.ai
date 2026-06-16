// The single keydown arbiter: a focused ui-core widget owns typing. This pins the THREE pieces of that
// contract — (1) `keyChar` is a neutral printable classifier (no behavior branch), (2) the engine forwards
// a key to the runtime and relays consumption, and (3) App.svelte forwards the key FIRST + ORs uiHasFocus
// into the `typing` predicate (source-level, the thin-shell-endstate style: a leak there fails the build).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { ShapeCanvasEngine } from "../renderer/engine";
import { keyChar } from "../controller/shortcuts";
import type { RustInputBatchResult, RustWebGpuRenderer, UiDispatchResult } from "../bridge/wasmLoader";
import type { CameraState } from "../renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

function canvasStub(): HTMLCanvasElement {
  return {
    width: 0,
    height: 0,
    addEventListener() {},
    removeEventListener() {},
    setPointerCapture() {},
    releasePointerCapture() {},
    getBoundingClientRect() {
      return { left: 0, top: 0, width: 800, height: 600 };
    }
  } as unknown as HTMLCanvasElement;
}
function overlayRoot(): HTMLElement {
  return { append() {} } as unknown as HTMLElement;
}

describe("keyChar classifies a printable insert vs a control key (no behavior branch)", () => {
  it("a bare printable returns the char; a control / modified key returns null", () => {
    expect(keyChar({ key: "a", ctrlKey: false, metaKey: false })).toBe("a");
    expect(keyChar({ key: " ", ctrlKey: false, metaKey: false })).toBe(" ");
    expect(keyChar({ key: "é", ctrlKey: false, metaKey: false })).toBe("é");
    // Control / navigation keys are multi-char names — never an insert.
    expect(keyChar({ key: "Backspace", ctrlKey: false, metaKey: false })).toBeNull();
    expect(keyChar({ key: "Enter", ctrlKey: false, metaKey: false })).toBeNull();
    expect(keyChar({ key: "ArrowLeft", ctrlKey: false, metaKey: false })).toBeNull();
    // A modified key is a shortcut, not an insert.
    expect(keyChar({ key: "a", ctrlKey: true, metaKey: false })).toBeNull();
    expect(keyChar({ key: "z", ctrlKey: false, metaKey: true })).toBeNull();
  });
});

describe("engine.uiKey forwards to the runtime and relays the verdict + actions", () => {
  function keyRenderer(consumed: boolean, actions: UiDispatchResult["actions"] = []): {
    renderer: RustWebGpuRenderer;
    lastKeyJson: () => string | null;
  } {
    let lastKeyJson: string | null = null;
    const renderer: RustWebGpuRenderer = {
      resize() {},
      renderFrame() {
        return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
      },
      uiKey(keyJson: string): UiDispatchResult {
        lastKeyJson = keyJson;
        return { consumed, sceneChanged: consumed, actions, edit: null };
      },
      inputBatch(): RustInputBatchResult {
        return { camera: CAMERA, objectDoubleClick: null };
      }
    };
    return { renderer, lastKeyJson: () => lastKeyJson };
  }

  function engineWith(renderer: RustWebGpuRenderer | null) {
    return new ShapeCanvasEngine({
      canvas: canvasStub(),
      overlayRoot: overlayRoot(),
      backend: "test",
      webGpuRenderer: renderer,
      onEvent: () => {}
    });
  }

  it("a focused UI field consumes the key (forwarded as neutral KeyInput JSON with modifier flags)", () => {
    const { renderer, lastKeyJson } = keyRenderer(true, [{ type: "textChanged", id: "demo-input", text: "ab" }]);
    const engine = engineWith(renderer);
    const result = engine.uiKey({ key: "b", text: "b", ctrl: false, meta: false, alt: false });
    expect(result).toEqual({ consumed: true, sceneChanged: true });
    expect(JSON.parse(lastKeyJson() ?? "{}")).toEqual({ key: "b", text: "b", ctrl: false, meta: false, alt: false });
  });

  it("a shortcut chord forwards its modifier flags (so the core can let undo/copy fall through)", () => {
    const { renderer, lastKeyJson } = keyRenderer(false);
    const engine = engineWith(renderer);
    // Cmd+Z: text is null (a modified key inserts nothing); the meta flag must survive the forward.
    engine.uiKey({ key: "z", text: null, ctrl: false, meta: true, alt: false });
    expect(JSON.parse(lastKeyJson() ?? "{}")).toEqual({ key: "z", text: null, ctrl: false, meta: true, alt: false });
  });

  it("with no focused UI field the key is NOT consumed (the catalog dispatcher still runs)", () => {
    const { renderer } = keyRenderer(false);
    const engine = engineWith(renderer);
    expect(engine.uiKey({ key: "g", text: "g", ctrl: false, meta: false, alt: false })?.consumed).toBe(false);
  });

  it("a renderer without uiKey returns null (the arbiter then falls through to the catalog)", () => {
    const renderer: RustWebGpuRenderer = {
      resize() {},
      renderFrame() {
        return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
      },
      inputBatch(): RustInputBatchResult {
        return { camera: CAMERA, objectDoubleClick: null };
      }
    };
    expect(engineWith(renderer).uiKey({ key: "a", text: "a", ctrl: false, meta: false, alt: false })).toBeNull();
    expect(engineWith(null).uiHasFocus()).toBe(false);
  });
});

describe("App.svelte routes the keydown through the single focus arbiter (source-level)", () => {
  const appSource = readFileSync(fileURLToPath(new URL("../ui/App.svelte", import.meta.url)), "utf8");

  it("the keydown forwards the key to uiKey FIRST, then preventDefault + return on consume", () => {
    // The UI forward sits BEFORE the Escape branch (so a focused field's Escape commits the field).
    const forward = appSource.indexOf("host?.uiKey({ key: event.key, text: keyChar(event),");
    const escape = appSource.indexOf('event.key === "Escape"');
    expect(forward, "the uiKey forward must exist").toBeGreaterThan(-1);
    expect(escape, "the Escape branch must exist").toBeGreaterThan(-1);
    expect(forward, "the uiKey forward must precede the Escape branch").toBeLessThan(escape);
    // On consume it swallows the key: preventDefault + return, no catalog dispatch.
    const guard = appSource.indexOf("if (uiKey?.consumed)", forward);
    expect(guard, "the consumed guard must follow the forward").toBeGreaterThan(forward);
  });

  it("uiHasFocus is ORed into the single `typing` predicate (no second focus check)", () => {
    // The same predicate the DOM-input check uses gains the UI-focus term — one arbiter, not two.
    expect(appSource).toContain("const typing = domTyping || (host?.uiHasFocus() ?? false)");
  });

  it("the uiKey forward branches on NO event.key literal (a neutral forward — the core decides)", () => {
    // The forward expression itself reads event.key/keyChar(event) verbatim; it must not gate behavior on a
    // key literal (e.g. `event.key === "Backspace"`) — the catalog stays the single binding source.
    const forwardLine = appSource
      .split("\n")
      .find((line) => line.includes("host?.uiKey({ key: event.key, text: keyChar(event),"));
    expect(forwardLine).toBeDefined();
    expect(forwardLine).not.toMatch(/event\.key\s*===/);
  });
});
