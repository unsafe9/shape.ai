import { describe, expect, it } from "vitest";
import {
  TextEditHost,
  type TextEditDocument,
  type TextEditEditable,
  type TextEditEvent,
  type TextEditRange,
  type TextEditSelection
} from "./textEditHost";

// A fake DOM seam so the IME decision logic is testable in the node env (no jsdom),
// matching the shell's existing browser-seam injection discipline. The fake records
// the caret/range plumbing and exposes a `fire` to drive listeners like a real event.
function fakeDom(): {
  doc: TextEditDocument;
  overlayRoot: { append(node: TextEditEditable): void; appended: TextEditEditable[] };
  editable(): TextEditEditable;
  fire(type: "input" | "keydown" | "blur", event?: Partial<TextEditEvent>): void;
  caretCollapsedToEnd: () => boolean;
  setActive(el: TextEditEditable | null): void;
} {
  const listeners = new Map<string, ((event: TextEditEvent) => void)[]>();
  let activeElement: { blur?: () => void; isContentEditable?: boolean } | null = null;
  let collapsedToStart: boolean | null = null;
  let created: TextEditEditable | null = null;

  const range: TextEditRange = {
    selectNodeContents() {},
    collapse(toStart: boolean) {
      collapsedToStart = toStart;
    }
  };
  const selection: TextEditSelection = { removeAllRanges() {}, addRange() {} };

  const makeEditable = (): TextEditEditable => {
    const el: TextEditEditable = {
      textContent: "",
      contentEditable: "",
      className: "",
      role: "",
      ariaLabel: "",
      tabIndex: -1,
      style: {},
      isContentEditable: true,
      focus() {
        activeElement = el;
      },
      // Mirror the browser: .blur() drops focus AND dispatches a `blur` event to listeners.
      blur() {
        if (activeElement === el) activeElement = null;
        for (const listener of listeners.get("blur") ?? []) listener(baseEvent({}));
      },
      remove() {},
      addEventListener(type, listener) {
        const list = listeners.get(type) ?? [];
        list.push(listener);
        listeners.set(type, list);
      }
    };
    return el;
  };

  const doc: TextEditDocument = {
    createElement() {
      created = makeEditable();
      return created;
    },
    getSelection: () => selection,
    createRange: () => range,
    get activeElement() {
      return activeElement;
    }
  };

  const overlayRoot = {
    appended: [] as TextEditEditable[],
    append(node: TextEditEditable) {
      overlayRoot.appended.push(node);
    }
  };

  const baseEvent = (overrides: Partial<TextEditEvent>): TextEditEvent => ({
    preventDefault() {},
    stopPropagation() {},
    ...overrides
  });

  return {
    doc,
    overlayRoot,
    editable: () => {
      if (!created) throw new Error("no editable created yet");
      return created;
    },
    fire(type, event = {}) {
      for (const listener of listeners.get(type) ?? []) listener(baseEvent(event));
    },
    caretCollapsedToEnd: () => collapsedToStart === false,
    setActive: (el) => {
      activeElement = el as { blur?: () => void; isContentEditable?: boolean } | null;
    }
  };
}

const RECT = { x: 10, y: 20, width: 100, height: 30 };

describe("TextEditHost — shared IME host-port library", () => {
  it("Enter outside composition commits the current textContent (single string back)", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const committed: string[] = [];
    host.open({ rect: RECT, value: "" }, { onInput: () => {}, onCommit: (v) => committed.push(v) });

    dom.editable().textContent = "hello";
    dom.fire("keydown", { key: "Enter", shiftKey: false, isComposing: false });

    expect(committed).toEqual(["hello"]);
    expect(host.isEditing()).toBe(false);
  });

  it("Enter / Escape DURING composition commit NOTHING (the isComposing guard)", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const committed: string[] = [];
    host.open({ rect: RECT, value: "" }, { onInput: () => {}, onCommit: (v) => committed.push(v) });

    dom.editable().textContent = "한";
    dom.fire("keydown", { key: "Enter", shiftKey: false, isComposing: true });
    dom.fire("keydown", { key: "Escape", isComposing: true });

    // FAILS if the guard regresses (a mid-composition Enter/Escape truncates the IME run).
    expect(committed).toEqual([]);
    expect(host.isEditing()).toBe(true);
  });

  it("blur fires onCommit with the live value", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const committed: string[] = [];
    host.open({ rect: RECT, value: "seed" }, { onInput: () => {}, onCommit: (v) => committed.push(v) });

    dom.editable().textContent = "typed away";
    dom.fire("blur");

    expect(committed).toEqual(["typed away"]);
  });

  it("an explicit Enter commit does not double-fire onCommit when blur follows", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const committed: string[] = [];
    host.open({ rect: RECT, value: "" }, { onInput: () => {}, onCommit: (v) => committed.push(v) });

    dom.editable().textContent = "once";
    dom.fire("keydown", { key: "Enter", shiftKey: false, isComposing: false });
    // The commit blurred the field; a trailing blur event must not commit a second time.
    dom.fire("blur");

    expect(committed).toEqual(["once"]);
  });

  it("blurActive() blurs a focused editable synchronously (commit survives a later preventDefault)", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const committed: string[] = [];
    host.open({ rect: RECT, value: "" }, { onInput: () => {}, onCommit: (v) => committed.push(v) });
    dom.editable().textContent = "click-away";

    // The active element IS this host's editable (focused on open); blurActive must blur it,
    // firing the committing onblur BEFORE the caller's preventDefault could suppress it.
    host.blurActive();

    expect(committed).toEqual(["click-away"]);
  });

  it("blurActive() is a no-op when no editable is focused", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const committed: string[] = [];
    host.open({ rect: RECT, value: "" }, { onInput: () => {}, onCommit: (v) => committed.push(v) });
    // Move focus away from the editable, then a non-editable element is active.
    dom.setActive({ isContentEditable: false, blur: () => {} });

    host.blurActive();

    expect(committed).toEqual([]);
  });

  it("seeds the value and places the caret at the END of the seeded text", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    host.open({ rect: RECT, value: "seeded" }, { onInput: () => {}, onCommit: () => {} });

    expect(dom.editable().textContent).toBe("seeded");
    expect(dom.caretCollapsedToEnd()).toBe(true);
  });

  it("reposition writes only left/top/width/height and never re-seeds textContent", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const handle = host.open({ rect: RECT, value: "keep" }, { onInput: () => {}, onCommit: () => {} });
    dom.editable().textContent = "edited";

    handle.reposition({ x: 99, y: 88, width: 77, height: 66 });

    const el = dom.editable();
    expect(el.style.left).toBe("99px");
    expect(el.style.top).toBe("88px");
    expect(el.style.width).toBe("77px");
    expect(el.style.height).toBe("66px");
    // Transform-only: the live edit text is untouched (no caret reset on every pan frame).
    expect(el.textContent).toBe("edited");
  });

  it("reuses ONE editable element across opens (never two divergent overlays)", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const committed: string[] = [];
    host.open({ rect: RECT, value: "first" }, { onInput: () => {}, onCommit: (v) => committed.push(v) });
    const firstEl = dom.editable();
    dom.editable().textContent = "first-edit";

    // Opening a second edit commits the first and reuses the same element.
    host.open({ rect: RECT, value: "second" }, { onInput: () => {}, onCommit: () => {} });

    expect(committed).toEqual(["first-edit"]);
    expect(dom.overlayRoot.appended.length).toBe(1);
    expect(dom.editable()).toBe(firstEl);
  });
});
