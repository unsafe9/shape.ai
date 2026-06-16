// The shared OS text-edit / IME host-port library. It realizes the core's
// "edit here, this value, this rect" request (canvas `beginTextEdit` /
// CoreOverlayRequest, ui-core `EditRequest`) by mounting ONE OS editing surface
// and feeding composition/commit/cancel back as a neutral committed string. It is
// a pure shell host-port: it authors no op, holds no canonical state, and decides
// nothing about the canvas or the UI — it only owns the browser-DOM IME mechanics
// (the composition guard, caret seeding, the blur-before-preventDefault hazard,
// plaintext-only) that are IDENTICAL for both surfaces. The two consumers adapt
// their own core descriptor into a neutral `TextEditMount` and pass two opaque
// callbacks; the library never knows which consumer it serves.

// A neutral mount target — the screen-px superset of the canvas overlay rect and
// the ui-core EditRequest rect. `rect` is SCREEN px (the canvas adapter does the
// world->screen derive before calling open; ui-core rects are already screen px).
export type TextEditMount = {
  rect: { x: number; y: number; width: number; height: number };
  value: string;
};

// The two opaque callbacks. `onInput` rides every live edit (the consumer mirrors
// it); `onCommit` rides the single committed string (Enter outside composition, or
// blur). The library calls back a STRING only — never a decision.
export type TextEditCallbacks = {
  onInput: (value: string) => void;
  onCommit: (value: string) => void;
};

// The handle returned by `open`. `reposition` is transform-only (left/top/width/
// height, never re-seeding textContent, so the caret never resets under pan/zoom);
// `close` tears the surface down without committing.
export type TextEditHandle = {
  reposition: (rect: TextEditMount["rect"]) => void;
  close: () => void;
};

// The minimal DOM seam the host needs. A real browser passes `document`; a node
// unit test passes a fake so the IME decision logic is testable without jsdom —
// the same injection discipline the rest of the shell uses for browser seams.
export type TextEditDocument = {
  createElement(tag: "div"): TextEditEditable;
  getSelection(): TextEditSelection | null;
  createRange(): TextEditRange;
  readonly activeElement: { blur?: () => void } | null;
};

// The subset of a contenteditable element the host drives. `textContent` is the
// live edited string; the listeners are the IME/commit wiring.
export type TextEditEditable = {
  textContent: string | null;
  contentEditable: string;
  className: string;
  role: string;
  ariaLabel: string;
  tabIndex: number;
  style: Record<string, string>;
  focus(): void;
  blur(): void;
  remove(): void;
  isContentEditable?: boolean;
  addEventListener(type: "input" | "keydown" | "blur", listener: (event: TextEditEvent) => void): void;
};

export type TextEditEvent = {
  isComposing?: boolean;
  key?: string;
  shiftKey?: boolean;
  preventDefault(): void;
  stopPropagation(): void;
};

export type TextEditSelection = {
  removeAllRanges(): void;
  addRange(range: TextEditRange): void;
};

export type TextEditRange = {
  selectNodeContents(node: TextEditEditable): void;
  collapse(toStart: boolean): void;
};

export type TextEditHostOptions = {
  // Where the editing surface is mounted (the renderer overlay root).
  overlayRoot: { append(node: TextEditEditable): void };
  // The shell's visual class for the surface (positioning + theme); the library owns
  // only the IME mechanics, not the look. Empty by default.
  className?: string;
  // Accessible name for the editing surface (role=textbox). Empty by default.
  ariaLabel?: string;
  // The DOM seam; defaults to the real browser `document` when omitted.
  doc?: TextEditDocument;
};

export class TextEditHost {
  private readonly overlayRoot: TextEditHostOptions["overlayRoot"];
  private readonly className: string;
  private readonly ariaLabel: string;
  private readonly doc: TextEditDocument;
  // The single reused editing surface; created lazily on first open, then reused so
  // there are never two divergent overlays. Null until first open.
  private editable: TextEditEditable | null = null;
  // The active edit's committed-once latch + its commit callback. `committed` guards
  // the blur->onCommit from firing twice when an explicit commit already ran.
  private active: { cb: TextEditCallbacks; committed: boolean } | null = null;

  constructor(options: TextEditHostOptions) {
    this.overlayRoot = options.overlayRoot;
    this.className = options.className ?? "";
    this.ariaLabel = options.ariaLabel ?? "";
    const injected = options.doc ?? (typeof document !== "undefined" ? (document as unknown as TextEditDocument) : null);
    if (!injected) throw new Error("TextEditHost requires a document (no DOM available)");
    this.doc = injected;
  }

  // True while an editing surface is mounted and focused.
  isEditing(): boolean {
    return this.active !== null;
  }

  // Mount the OS editing surface at the mount rect, seed its value, focus it with the
  // caret at the end, and wire IME-guarded commit + live input. Opening a new edit
  // commits any prior one first (single active edit; never two concurrent overlays).
  open(mount: TextEditMount, cb: TextEditCallbacks): TextEditHandle {
    if (this.active) this.commit();
    const el = this.ensureEditable();
    this.active = { cb, committed: false };

    this.applyRect(el, mount.rect);
    // Seed value + caret-at-end (the lifted mountTextEdit logic). Seeding the
    // textContent BEFORE focus so the caret-at-end range sees the full text.
    el.textContent = mount.value;
    el.focus();
    this.placeCaretAtEnd(el);

    return {
      reposition: (rect) => this.applyRect(el, rect),
      close: () => this.dismiss()
    };
  }

  // The blur-before-preventDefault hazard, lifted out of engine.ts so it lives in ONE
  // place for both surfaces: a canvas mousedown calls preventDefault to suppress native
  // selection/image-drag, which ALSO suppresses the browser blur of a focused editable —
  // so the committing onblur never fires. Blurring synchronously BEFORE preventDefault
  // commits the edit (preventDefault does not cancel an explicit .blur()). No-op when the
  // active element is not this host's editable.
  blurActive(): void {
    const active = this.doc.activeElement;
    if (active && (active as { isContentEditable?: boolean }).isContentEditable && typeof active.blur === "function") {
      active.blur();
    }
  }

  // ---- internals ----

  private ensureEditable(): TextEditEditable {
    if (this.editable) return this.editable;
    const el = this.doc.createElement("div");
    el.contentEditable = "plaintext-only";
    el.className = this.className;
    el.role = "textbox";
    el.ariaLabel = this.ariaLabel;
    el.tabIndex = 0;
    // Live input: report the whole current string. The consumer mirrors it; the
    // library computes nothing from it.
    el.addEventListener("input", () => {
      if (this.active) this.active.cb.onInput(el.textContent ?? "");
    });
    // The isComposing guard: Enter/Escape are swallowed mid-composition (an IME
    // sequence is in flight — committing or canceling there would truncate it).
    // Outside composition, Enter (no Shift) and Escape commit the current text.
    el.addEventListener("keydown", (event) => {
      if (event.isComposing) return;
      if ((event.key === "Enter" && !event.shiftKey) || event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        this.commit();
      }
    });
    // A blur (click-away / programmatic .blur()) commits the live value.
    el.addEventListener("blur", () => this.commit());
    this.overlayRoot.append(el);
    this.editable = el;
    return el;
  }

  // Commit the live value through the active callback exactly once, then tear the
  // surface down. The `committed` latch keeps the blur that follows an explicit
  // Enter/Escape commit from firing a second onCommit.
  private commit(): void {
    const active = this.active;
    if (!active || active.committed) return;
    active.committed = true;
    const value = this.editable?.textContent ?? "";
    this.dismiss();
    active.cb.onCommit(value);
  }

  // Drop the active edit WITHOUT committing (consumer-driven close). The element is
  // kept for reuse; only the value + focus are cleared.
  private dismiss(): void {
    this.active = null;
    if (this.editable) {
      this.editable.textContent = "";
      this.editable.blur();
    }
  }

  // Transform-only restyle so the surface tracks pan/zoom with zero rebuild — never
  // re-seeds textContent (that would reset the caret every frame under pan).
  private applyRect(el: TextEditEditable, rect: TextEditMount["rect"]): void {
    el.style.left = `${rect.x}px`;
    el.style.top = `${rect.y}px`;
    el.style.width = `${rect.width}px`;
    el.style.height = `${rect.height}px`;
  }

  private placeCaretAtEnd(el: TextEditEditable): void {
    const selection = this.doc.getSelection();
    if (!selection) return;
    const range = this.doc.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
    selection.removeAllRanges();
    selection.addRange(range);
  }
}
