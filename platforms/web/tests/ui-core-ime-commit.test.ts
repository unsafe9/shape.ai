// The ui-core IME consumer (App.svelte `handleUiEdit`) contract: while the shared OS editing surface is
// mounted, that surface is the SOLE writer for the focused ui-core field. It owns CJK composition natively
// and hands back ONE finished string on commit, which the shell lands wholesale via engine.uiCommitText.
// This pins the central hazard the review flagged: a per-keystroke relay (the old `forwardUiText`) doubled
// composing jamo into the core ("ㅎ하한"); these assertions FAIL if any per-key relay re-appears OR if the
// commit forwards anything but the single composed syllable.
import { describe, expect, it } from "vitest";

import { ShapeCanvasEngine } from "../renderer/engine";
import { TextEditHost, type TextEditDocument, type TextEditEditable, type TextEditEvent } from "../ime/textEditHost";
import type { RustInputBatchResult, RustWebGpuRenderer, UiDispatchResult } from "../bridge/wasmLoader";
import type { CameraState } from "../renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };
const RECT = { x: 10, y: 20, width: 200, height: 32 };

// A renderer whose ui-core runtime models the real `commit_text`: it sets the focused field to the one
// committed value (NOT a per-key append) and records every uiKey / uiCommitText call so the test can prove
// which seam the composition flowed through.
function uiTextRenderer(): {
  renderer: RustWebGpuRenderer;
  uiKeyTexts: () => (string | null)[];
  committed: () => string[];
  fieldValue: () => string;
} {
  const uiKeyTexts: (string | null)[] = [];
  const committed: string[] = [];
  let fieldValue = "";
  const renderer: RustWebGpuRenderer = {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    uiKey(keyJson: string): UiDispatchResult {
      const key = JSON.parse(keyJson) as { text: string | null };
      uiKeyTexts.push(key.text);
      // Mirror the core's per-key append (the path that MUST NOT carry composition).
      if (key.text) fieldValue += key.text;
      return { consumed: true, sceneChanged: true, actions: [], edit: null };
    },
    uiCommitText(value: string): UiDispatchResult {
      committed.push(value);
      // Mirror the core's commit_text: set the field wholesale, then blur.
      fieldValue = value;
      return { consumed: true, sceneChanged: true, actions: [], edit: null };
    },
    inputBatch(): RustInputBatchResult {
      return { camera: CAMERA, objectDoubleClick: null };
    }
  };
  return { renderer, uiKeyTexts: () => uiKeyTexts, committed: () => committed, fieldValue: () => fieldValue };
}

// The minimal DOM seam the host needs, with a `fire` to drive composition input events.
function fakeDom(): {
  doc: TextEditDocument;
  overlayRoot: { append(node: TextEditEditable): void };
  editable(): TextEditEditable;
  fire(type: "input" | "keydown" | "blur", event?: Partial<TextEditEvent>): void;
} {
  const listeners = new Map<string, ((event: TextEditEvent) => void)[]>();
  let created: TextEditEditable | null = null;
  let active: TextEditEditable | null = null;
  const make = (): TextEditEditable => ({
    textContent: "",
    contentEditable: "",
    className: "",
    role: "",
    ariaLabel: "",
    tabIndex: -1,
    style: {},
    isContentEditable: true,
    focus() {
      active = created;
    },
    blur() {
      if (active === created) active = null;
      for (const l of listeners.get("blur") ?? []) l({ preventDefault() {}, stopPropagation() {} });
    },
    remove() {},
    addEventListener(type, listener) {
      const list = listeners.get(type) ?? [];
      list.push(listener);
      listeners.set(type, list);
    }
  });
  const doc: TextEditDocument = {
    createElement() {
      created = make();
      return created;
    },
    getSelection: () => ({ removeAllRanges() {}, addRange() {} }),
    createRange: () => ({ selectNodeContents() {}, collapse() {} }),
    get activeElement() {
      return active;
    }
  };
  return {
    doc,
    overlayRoot: { append() {} },
    editable: () => {
      if (!created) throw new Error("no editable yet");
      return created;
    },
    fire(type, event = {}) {
      for (const l of listeners.get(type) ?? []) l({ preventDefault() {}, stopPropagation() {}, ...event });
    }
  };
}

function engineWith(renderer: RustWebGpuRenderer): ShapeCanvasEngine {
  return new ShapeCanvasEngine({
    canvas: {
      width: 0,
      height: 0,
      addEventListener() {},
      removeEventListener() {},
      setPointerCapture() {},
      releasePointerCapture() {},
      getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 })
    } as unknown as HTMLCanvasElement,
    overlayRoot: { append() {} } as unknown as HTMLElement,
    backend: "test",
    webGpuRenderer: renderer,
    onEvent: () => {}
  });
}

describe("ui-core IME consumer: the OS surface is the sole writer; commit lands ONE composed string", () => {
  it("composing 한 in place commits exactly '한' with NO per-jamo uiKey relay", () => {
    const { renderer, uiKeyTexts, committed, fieldValue } = uiTextRenderer();
    const engine = engineWith(renderer);
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });

    // Wire EXACTLY as App.svelte `handleUiEdit` does: onInput is a no-op (the OS surface holds the live
    // text); onCommit lands the one finished string via engine.uiCommitText.
    host.open(
      { rect: RECT, value: "" },
      { onInput: () => {}, onCommit: (value) => engine.uiCommitText(value) }
    );

    // The browser mutates the composing syllable IN PLACE: ㅎ -> 하 -> 한, firing input each step. A
    // suffix-diff relay would re-send the whole buffer per step (the "ㅎ하한" corruption); here onInput
    // must forward NOTHING to the core.
    for (const intermediate of ["ㅎ", "하", "한"]) {
      dom.editable().textContent = intermediate;
      dom.fire("input");
    }
    // Commit outside composition (Enter): the library hands back the one finished string.
    dom.fire("keydown", { key: "Enter", shiftKey: false, isComposing: false });

    expect(committed(), "exactly one committed syllable reaches the core").toEqual(["한"]);
    expect(uiKeyTexts(), "no composing keystroke is relayed per-key into the core").toEqual([]);
    expect(fieldValue(), "the core field holds '한', never the doubled 'ㅎ하한'").toBe("한");
    expect(host.isEditing()).toBe(false);
  });

  it("a committed value that REPLACED in place (not a prefix extension) still lands wholesale", () => {
    // CJK composition can decompose/replace: the committed string is not seed+suffix. Wholesale commit is
    // correct here where the old append-only suffix diff would have corrupted the field.
    const { renderer, committed, fieldValue } = uiTextRenderer();
    const engine = engineWith(renderer);
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    host.open(
      { rect: RECT, value: "가" },
      { onInput: () => {}, onCommit: (value) => engine.uiCommitText(value) }
    );

    dom.editable().textContent = "나"; // replaced the seed, not appended to it
    dom.fire("keydown", { key: "Enter", shiftKey: false, isComposing: false });

    expect(committed()).toEqual(["나"]);
    expect(fieldValue()).toBe("나");
  });
});

describe("engine.uiCommitText forwards the one committed string and relays the TextChanged", () => {
  it("forwards the value verbatim and surfaces the resulting action as a ui-action event", () => {
    const events: { type: string }[] = [];
    let received: string | null = null;
    const renderer: RustWebGpuRenderer = {
      resize() {},
      renderFrame() {
        return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
      },
      uiCommitText(value: string): UiDispatchResult {
        received = value;
        return {
          consumed: true,
          sceneChanged: true,
          actions: [{ type: "textChanged", id: "demo-input", text: value }],
          edit: null
        };
      },
      inputBatch(): RustInputBatchResult {
        return { camera: CAMERA, objectDoubleClick: null };
      }
    };
    const engine = new ShapeCanvasEngine({
      canvas: {
        width: 0,
        height: 0,
        addEventListener() {},
        removeEventListener() {},
        setPointerCapture() {},
        releasePointerCapture() {},
        getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 })
      } as unknown as HTMLCanvasElement,
      overlayRoot: { append() {} } as unknown as HTMLElement,
      backend: "test",
      webGpuRenderer: renderer,
      onEvent: (event) => events.push(event)
    });

    const result = engine.uiCommitText("한글");
    expect(result).toEqual({ consumed: true, sceneChanged: true });
    expect(received).toBe("한글");
    const action = events.find((e) => e.type === "ui-action");
    expect(action).toEqual({ type: "ui-action", action: { type: "textChanged", id: "demo-input", text: "한글" } });
  });

  it("returns null when the renderer predates uiCommitText (graceful feature-detect)", () => {
    const renderer: RustWebGpuRenderer = {
      resize() {},
      renderFrame() {
        return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
      },
      inputBatch(): RustInputBatchResult {
        return { camera: CAMERA, objectDoubleClick: null };
      }
    };
    const engine = new ShapeCanvasEngine({
      canvas: {
        width: 0,
        height: 0,
        addEventListener() {},
        removeEventListener() {},
        setPointerCapture() {},
        releasePointerCapture() {},
        getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 })
      } as unknown as HTMLCanvasElement,
      overlayRoot: { append() {} } as unknown as HTMLElement,
      backend: "test",
      webGpuRenderer: renderer,
      onEvent: () => {}
    });
    expect(engine.uiCommitText("x")).toBeNull();
  });
});
