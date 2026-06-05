# Editing Boundary Confirmation

## Decision

- Normal canvas objects stay renderer scene objects.
- Active text editing uses a DOM textarea overlay.
- Selected-card toolbars, context menus, inspectors, comments, and export UI remain TypeScript DOM shell responsibilities.
- Full DOM card rendering and live Web Component canvas are not the replacement path.

## Implemented Flow

1. Hit testing returns a stable object id plus optional text field.
2. Double-clicking card text calls `beginTextEdit`.
3. The engine computes a world rect and screen rect for the target field.
4. TypeScript mounts a DOM `<textarea>` over the canvas.
5. Native browser editing handles focus, selection, copy/paste, and IME.
6. Blur or `Cmd/Ctrl+Enter` commits an `edit-card-text` scene patch.
7. The renderer updates text cache and redraws the card.

## Verification Checklist

- Multiple zoom levels: overlay tracks the same world rect via shared camera transform.
- Korean IME: should work because the active editor is a native textarea.
- Copy/paste/selection: should work because editing is not reimplemented in canvas.
- Commit path: renderer patch is separate from business validation.

## Remaining Manual Check

Run:

```bash
npm run poc:dev
```

Open `http://127.0.0.1:5174`, double-click a card title or summary, type Korean text, zoom, then commit. Record whether the overlay alignment remains acceptable.
