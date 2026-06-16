import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { TextEditHost, type TextEditDocument, type TextEditEditable, type TextEditEvent } from "../ime/textEditHost";

// Locked product decision: Escape in the inline text-edit overlay COMMITS the typed text (exit edit,
// keep content) — it does not discard it. The overlay's IME mechanics now live in the shared TextEditHost
// library, so this drives the REAL library: Escape outside composition commits the live value (which
// App.svelte authors as a set-text op), and Escape DURING composition commits nothing. Re-pointed from the
// old App.svelte onkeydown regex-lift after the logic moved into ime/textEditHost.ts.

const appSource = readFileSync(fileURLToPath(new URL("../ui/App.svelte", import.meta.url)), "utf8");

function fakeDom(): {
  doc: TextEditDocument;
  overlayRoot: { append(node: TextEditEditable): void };
  editable(): TextEditEditable;
  fire(type: "keydown", event: Partial<TextEditEvent>): void;
} {
  const listeners = new Map<string, ((event: TextEditEvent) => void)[]>();
  let created: TextEditEditable | null = null;
  const makeEditable = (): TextEditEditable => ({
    textContent: "",
    contentEditable: "",
    className: "",
    role: "",
    ariaLabel: "",
    tabIndex: -1,
    style: {},
    isContentEditable: true,
    focus() {},
    blur() {
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
      created = makeEditable();
      return created;
    },
    getSelection: () => ({ removeAllRanges() {}, addRange() {} }),
    createRange: () => ({ selectNodeContents() {}, collapse() {} }),
    get activeElement() {
      return null;
    }
  };
  return {
    doc,
    overlayRoot: { append() {} },
    editable: () => {
      if (!created) throw new Error("no editable");
      return created;
    },
    fire(type, event) {
      for (const l of listeners.get(type) ?? []) l({ preventDefault() {}, stopPropagation() {}, ...event });
    }
  };
}

describe("inline text commit on Escape", () => {
  it("commits the typed text on Escape (set-text preserves the buffer, not discarded)", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });

    // The App-level commit path: a committed value authors a set-text op carrying the buffer (no discard).
    const authored: unknown[] = [];
    const commitTextEdit = (value: string) =>
      authored.push({ kind: "set-text", id: "text-7", text: { runs: [{ text: value }] } });

    host.open({ rect: { x: 0, y: 0, width: 10, height: 10 }, value: "" }, { onInput: () => {}, onCommit: commitTextEdit });
    dom.editable().textContent = "hello world";
    dom.fire("keydown", { key: "Escape", isComposing: false });

    // Escape committed: a set-text op carrying the typed buffer was authored (NOT discarded).
    expect(authored).toEqual([{ kind: "set-text", id: "text-7", text: { runs: [{ text: "hello world" }] } }]);
    expect(host.isEditing()).toBe(false);
  });

  it("ignores Escape mid-composition (IME), committing nothing", () => {
    const dom = fakeDom();
    const host = new TextEditHost({ overlayRoot: dom.overlayRoot, doc: dom.doc });
    const authored: string[] = [];
    host.open(
      { rect: { x: 0, y: 0, width: 10, height: 10 }, value: "" },
      { onInput: () => {}, onCommit: () => authored.push("commit") }
    );

    dom.editable().textContent = "한";
    dom.fire("keydown", { key: "Escape", isComposing: true });

    expect(authored).toEqual([]);
    expect(host.isEditing()).toBe(true);
  });

  it("App.svelte's Escape-while-editing blurs the active surface so the library commits the live value", () => {
    // The window-level Escape fallback routes through the shared library's blurActive (which commits the
    // live editing surface), never the stale shell mirror. FAILS if the fallback reverts to a direct commit
    // of textEdit.value or drops the inline-edit branch.
    const handleEscape = appSource.indexOf("function handleEscape");
    const body = appSource.slice(handleEscape, appSource.indexOf("function setActiveTool", handleEscape));
    expect(body, "Escape-while-editing must commit via the IME library").toContain(
      "if (textEdit) return void textEditHost?.blurActive();"
    );
  });
});
