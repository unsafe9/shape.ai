# UI redesign — macOS-native, translucent (vibrancy)

Locked design for a full visual redesign of the Rust-rendered product UI
(`crates/ui-core` primitives + `crates/ui` surfaces). Goal: replace the current
flat, text-label, disconnected-box toolbar and panels with a clean macOS-native
look — icon buttons, a single rounded translucent "material" tray, hairline
borders, soft elevation shadows, and frosted backdrop-blur (vibrancy).

The cores stay portable and token-driven: a theme flip must remain a recolor with
**zero geometry/fill rebake** (the `theme_flip_*_no_rebake` tests stay green).
Performance is the top rule — nothing here adds per-frame re-tessellation; the only
GPU addition (backdrop blur) is panel-scoped and gated on a world-dirty bit.

## Invariants (must stay green)

- Every toolbar/menu `cmd:<id>` maps to an `object_command_catalog` id
  (`toolbar_ids_are_all_in_the_command_catalog`). Settings = one row per catalog
  entry. Do not add/rename commands; icons are a presentation concern only.
- All paints stay theme **Token**s (or deliberate literals already present: peer
  colors, pen swatches, the connection dot, danger-red). No baked theme color in
  geometry/fill → keeps `theme_flip_*_no_rebake` green.
- Preserve every widget id / prefix the intent resolver depends on
  (`cmd:`/`swatch:`/`pen-width:`/`template:`/`canvas:`/`canvas-delete:`/
  `canvas-new`/`insp:` + scrim ids). Restyle is geometry/paint only.
- No product UI returns to the DOM (thin-shell guard). All chrome stays Rust.

## Tokens (add to BOTH `scene-core/.../catalog/theme.rs` and
`renderer-core/object_theme.rs`, byte-identical RGBA; bump `ALL_TOKENS` len +
`name()`/`rgba()` arms). Text-only tokens ALSO go in `ui-core/theme.rs::token_hex`
+ its mirror-test list.

| token            | light (rgba8) | dark (rgba8) | use |
|------------------|---------------|--------------|-----|
| `material`       | `f7f7f9e6`    | `2c2c2ee6`   | panel/tray fill (translucent; frosted w/ blur) |
| `hairline`       | `0000001f`    | `ffffff26`   | 1px borders + separators |
| `hover`          | `00000014`    | `ffffff1f`   | hover background |
| `accent-soft`    | `007aff26`    | `0a84ff3d`   | active/selected background tint |
| `text-secondary` | `8a8a8eff`    | `98989dff`   | muted labels / units (text token) |

Reuse existing: `selection-ring` (#007aff/#0a84ff) = accent (icon tint when
active); `surface`/`surface-muted` = control fills; `text` = primary text;
`canvas-bg` unchanged. `text-secondary` is the only NEW text token → add to
`token_hex` + the `text_paint_token_matches_object_theme` list.

## ui-core additions (foundation)

1. `Icon` widget: `{ id, x, y, w, h, d: String (SVG-subset path in a 24×24 box),
   fill: Option<Paint>, stroke: Option<(Paint, f64)> }`. `emit_icon` scales the
   24×24 path to the icon box and quantizes (q at 8u/px, absolute M/L/C/Z only).
   Most icons are STROKED line-art (SF-Symbols feel): thin stroke (~1.6px @24box),
   round cap/join. No renderer change — rides the existing path+fill+stroke seam.
2. `RectStyle.opacity: f64` (default 1.0) threaded into `RFill.opacity` in
   `rect_object` (render.rs currently hardcodes 1.0). Needed for translucent SOLID
   shadow rects (`RPaint::Solid` carries no alpha).
3. Soft-shadow helper (ui crate, from primitives only): under each ELEVATED panel
   emit 2–3 stacked rounded `Rect`s, `Solid` black, increasing (offset, size),
   decreasing opacity (~0.12→0.04 light / ~0.40→0.12 dark), to fake a Gaussian
   falloff. Tune by eye. No renderer change.

## Icon set (24×24 stroke line-art; author in ui crate icon registry keyed by
command id — presentation, NOT the scene-core catalog)

Toolbar (critical): `select-move`=NW arrow cursor, `hand-pan`=hand, `draw`=pencil,
`erase`=eraser block, `insert-rectangle`=square, `insert-ellipse`=circle,
`insert-line`=diagonal line, `insert-text`=`T`, `insert-frame`=frame w/ corner
ticks, `duplicate`=two overlapping squares, `delete`=trash, `group`=dashed box
over squares, `ungroup`=broken box, `undo`=curved arrow left, `redo`=curved arrow
right, `open-template-library`=2×2 grid, `export`=tray + up arrow,
`toggle-diagnostics`=activity pulse, `zoom-out`=magnifier −, `zoom-in`=magnifier
+, `zoom-fit`=corner-expand in frame, `toggle-fullscreen`=4-corner expand.
Chrome glyphs → icons too: `toggle-theme`=sun/moon, `canvas-new`=plus,
`canvas-delete`=trash. Keep each geometrically simple; a test asserts every
toolbar command id has a registered icon and each path parses (valid M/L/C/Z,
coords within 0..24).

## Surface specs (the macOS material language, applied to every surface)

Common panel: `material` fill, `hairline` 1px border, corner radius 12–14, soft
shadow underlay, inner padding 12–14. Active control = `accent-soft` bg +
`selection-ring` icon/label; hover = `hover` bg; muted captions/units =
`text-secondary`.

- **Toolbar** (`toolbar.rs`): ONE rounded `material` tray (radius 14), not 22 loose
  boxes. Icon buttons (icon-only, ~32×32, radius 8) in catalog order, with thin
  `hairline` vertical **separators** between groups (tool | insert | edit | history
  | more | view). Inline pen-width chips + color swatches stay but restyled to fit.
  Active tool/insert = `accent-soft` + accent icon (NOT today's full bright-blue
  fill). Hover = `hover`. Soft shadow under the tray. Centered bottom.
  Tooltip (stretch): on hover show a small dark pill with the catalog label above
  the button, IF the hovered widget id is exposed to `build_root`; else defer.
- **Inspector** (`inspector.rs`/`composites.rs`): material panel; section titles in
  `text`, control labels/units `text-secondary`; `button_style` → material/hover/
  accent-soft states + radius 8; inputs `surface-muted` bg + `hairline`; align grid
  cells get `hover`/`accent-soft` states. Keep all `insp:` ids + value readers.
- **Settings / Context menu / Template / Diagnostics / Status / Presence /
  Chrome (theme-toggle, canvas-switcher, watermark)**: same material/hairline/
  radius/shadow/secondary-text language. Theme toggle + canvas +/trash become
  icons. Keep all ids/prefixes + token paints.

## Backdrop blur (vibrancy) — FINAL, isolated renderer phase

The one architectural change. World today renders straight to the swapchain
(`RENDER_ATTACHMENT` only) → not sampleable. To frost panels:

1. Render the world to an offscreen `RENDER_ATTACHMENT | TEXTURE_BINDING` color
   target (retarget the shadow-composite + `renderer.render` + overlays in
   `frame.rs`), then blit/copy to the swapchain.
2. Before the UI pass, for each panel rect: sample the offscreen world, run the
   existing separable Gaussian (`shadow_blur.rs`, quarter-res, reuse pipelines/
   sampler/layout) **scoped to the panel rect**, composite under the (translucent
   `material`) panel with a rounded-rect mask + tint.
3. Perf: recompute a panel's blur only when content behind changed — gate on a
   world-dirty bit (camera moved or an object feed happened). Panels are
   screen-fixed (identity camera), so otherwise reuse last frame's blurred crop.

Graceful degrade: if disabled/unstable, the translucent `material` token alone
still reads as a (lighter) macOS panel — the redesign does not depend on blur.

## Out of scope

Canvas seed content ("Hello shape" + sample rects) is user content, not chrome.
World-object shadow behavior is unchanged. No new commands/gestures.

## Phases (one ultracode workflow; sequential impl in-place, no worktrees;
release-only builds; node off PATH so prefix web cmds with the node 24.4.1 keg;
use `make` wrappers)

1. Foundation — `Icon` widget + `emit_icon`, `RectStyle.opacity`, the 5 new tokens
   in all 3 tables, falsifiable tests.
2. Icon set — registry + per-command SVG paths + placement helper + parse/coverage
   test.
3. Toolbar — icon tray, separators, state tokens, soft shadow, (stretch) tooltip.
4. Surfaces A — inspector + settings + diagnostics + template (material language).
5. Surfaces B — context menu + presence + status + chrome (theme/canvas/watermark).
6. Backdrop blur — offscreen world target + panel-scoped gated blur.
7. Review (parallel adversarial) + fix + final gate
   (`make lint && make test-rust && make test-web && make build`).
