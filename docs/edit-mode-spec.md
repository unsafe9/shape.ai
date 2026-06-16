# Edit-Mode / Inspector Implementation Spec

Single source of truth for the edit-mode build (Free/Flow layout reshape, sizing,
canonical orientation, the inspector catalog + view). Every later phase reads this
file. Grounded in the current code as of this writing; all `file:line` refs are to
`/Users/wshan/workspace/shape.ai`.

## 0. Locked principles (do not deviate)

- **One visual state = one canonical representation.** Each concern owns one
  channel; the other is structurally blocked.
- **Position is owned by the parent container's mode.** A container is *Free*
  (`Object.layout == None`: children keep their own `transform`) or *Flow*
  (`Object.layout == Some(Layout)`: the parent arranges children). There is NO
  per-child absolute override inside a Flow container. Overlay/stack = a Free
  container nested as a Flow child. Keep `layout: Option<Layout>` (`None` == Free)
  — do NOT introduce a separate mode enum.
- **Orientation is owned by `transform`.** Geometry is stored canonically; the
  `Canonicalize` op extracts a geometry's dominant angle into `transform` and
  rewrites geometry axis-aligned. The inspector reads rotation/scale/skew from a
  transform affine decomposition (single source of truth). A genuinely freeform
  shape honestly reads rotation 0 (AABB selection box; no min-area OBB).
- **Pure cores.** No time/rng/threads/IO; pointer-width-agnostic (no
  width-narrowing `as`; coords i32, use `i64`/checked `try_from` where a count is
  accumulated). `wasm_api.rs` is a JSON bridge only.
- **Zero-rebake contract.** Transform edits never re-tessellate geometry. The
  layout solve is derived, never stored (`layout_solve.rs:1-6`). `Canonicalize`
  *does* rewrite geometry (it is a geometry edit), so it is a rebake op — that is
  intended and acceptable because it is a deliberate user action, not a per-frame
  path.

## 1. Module layout / where new code lands

Current tiers (`crates/scene-core/src/object/mod.rs:1-19`): `kernel <-
{authoring, binding, catalog}`, one-way.

| New code | File | Tier |
|---|---|---|
| Reshaped `Layout` + new model types (`LayoutAxis`, `Lanes`, `MainAlign`, `CrossAlign`, `Align`, `AxisSizing`, `Sizing`, `ObjectMetaFields` if used) | `crates/scene-core/src/object/kernel/model.rs` | kernel |
| New ops (`SetSizing`, `SetMeta`, `Canonicalize`) | `crates/scene-core/src/object/kernel/op.rs` | kernel |
| Apply + inverse | `crates/scene-core/src/object/kernel/apply.rs` | kernel |
| Validate | `crates/scene-core/src/object/kernel/validate.rs` | kernel |
| `decompose_affine` / `compose_affine` helpers + `Canonicalize` geometry math support | `crates/scene-core/src/object/binding/affine.rs` (NEW) | binding |
| Inspector catalog + dynamic view | `crates/scene-core/src/object/catalog/inspector.rs` (NEW) | catalog |
| wasm exports | `crates/scene-core/src/wasm_api.rs` | bridge |
| Re-exports | `crates/scene-core/src/object/mod.rs`, `crates/scene-core/src/object/binding/mod.rs`, `crates/scene-core/src/object/catalog/mod.rs` | facade |
| Golden tests | `crates/scene-core/tests/object_golden.rs` (and the existing `ob53_*` layout test rewritten) | tests |

Module registration:
- `crates/scene-core/src/object/binding/mod.rs:1-7` — add `pub mod affine;`.
- `crates/scene-core/src/object/catalog/mod.rs:1-3` — add `pub mod inspector;`.
- `crates/scene-core/src/object/mod.rs` — add `pub use binding::affine;` near
  line 14-16; add `pub use catalog::inspector;` near line 18; extend the
  `pub use affine::{...}` / `pub use inspector::{...}` / `pub use model::{...}` /
  `pub use op::{...}` blocks with the new symbols (see each section).

---

## 2. Model types to add / reshape

All wire types get, matching the existing `Paint`/`Layout` style
(`model.rs:496-505`):
```rust
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
```
plus `#[derive(Clone, ..., Serialize, Deserialize)]`. Enums that are `Copy` keep
`Copy, Eq, Hash` like the current layout enums (`model.rs:463-491`).

### 2.1 Reshape `Layout` (replaces `model.rs:466-505`)

DELETE the three old enums `LayoutDirection` (`model.rs:463-470`), `LayoutAlign`
(`model.rs:472-481`), `LayoutSizing` (`model.rs:483-491`) and the old `Layout`
struct (`model.rs:493-505`). Replace with:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum LayoutAxis {
    Horizontal,
    Vertical,
}

/// Count{value>=1}: 1 = single list track, N = N-track grid. Fill = wrap-as-fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Lanes {
    Count { value: u32 },
    Fill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum MainAlign {
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub enum CrossAlign {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Align {
    pub main: MainAlign,
    pub cross: CrossAlign,
}

/// Auto-layout inputs on a children group; output positions are derived (not
/// stored/synced). `spacing` (quantized) is BOTH the inter-child gap AND the
/// uniform container edge inset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub axis: LayoutAxis,
    pub lanes: Lanes,
    pub spacing: i32,
    pub align: Align,
}
```

Notes:
- `Layout` was `Clone, Debug, PartialEq, Eq` (not `Copy`) at `model.rs:495`. It is
  now all-`Copy` fields, so `Copy` is safe and lets `apply.rs` drop a `.clone()`
  (optional; keeping `.clone()` also compiles). The `SetLayout` op payload keeps
  `Option<Layout>` so its on-the-wire shape `{ id, layout }` is unchanged in
  shape; only the inner `Layout` fields changed.
- `Lanes` is `#[serde(tag="kind")]` (camelCase tag) just like `Paint`
  (`model.rs:314`) and `CommentAnchor` (`model.rs:510`); the `Count` value is
  `u32` (`value >= 1`, validated below).
- The OLD `Layout` had `direction/gap/padding/align/sizing`; ALL of those fields
  are GONE. `gap` and `padding` collapse into the single `spacing`. The old
  per-container `sizing: LayoutSizing` is replaced by per-object `Sizing` (below).

### 2.2 Per-object `Sizing` (new)

```rust
/// value quantized (object-local units).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AxisSizing {
    Hug,
    Fill,
    Fixed { value: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Sizing {
    pub w: AxisSizing,
    pub h: AxisSizing,
}
```
`Object` gains `sizing: Option<Sizing>` — `None` => default Hug/Hug.

### 2.3 Object header metadata (new) — DECISION

`Object.meta` is `Option<ObjectMeta>` where `ObjectMeta = serde_json::Map<String,
serde_json::Value>` (`model.rs:20`, `model.rs:589-591`). It is an UNTYPED escape
hatch and (confirmed by grep) is read by NOTHING outside the re-exports. The spec
requires `name`/`hidden`/`locked` to be *canonical synced typed* scene state with
default-skip-on-wire. Stuffing them into the untyped `meta` map would (a) lose
typing, (b) lose ts-rs derivation, (c) not default-skip cleanly.

**Decision: add three typed Object-level fields, NOT a nested struct, NOT `meta`.**
They sit alongside `clip`/`tags` (`model.rs:573-582`):

```rust
    /// Panel header display name; `None` => fall back to a derived label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Hidden from render + hit-test (panel eye toggle).
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Locked against selection/edit (panel lock toggle).
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
```
Add a free helper (module-private) used by both fields:
```rust
fn is_false(b: &bool) -> bool { !*b }
```
(ts-rs emits `boolean` for `bool` and `string | null`/optional for
`Option<String>` regardless of `skip_serializing_if`; the skip only affects the
serde wire, which is what we want — minimal on the wire, default `false`/`None`.)

`Object::new` (`model.rs:596-616`) initializes them: `name: None, hidden: false,
locked: false`.

### 2.4 Exact `Object` field placement

Current `Object` struct ends at `model.rs:589-591` with `meta`. Add `sizing`
adjacent to `layout` (it is the per-object layout-sizing partner) and the three
header fields after `tags`. Concrete diff intent (insert into the struct body):

- after `pub layout: Option<Layout>,` (`model.rs:573-574`) add:
  ```rust
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub sizing: Option<Sizing>,
  ```
- after `pub tags: Vec<String>,` (`model.rs:581-582`) add the `name`/`hidden`/
  `locked` block from 2.3.

Update `Object::new` (`model.rs:596-616`) to set `sizing: None, name: None,
hidden: false, locked: false`.

### 2.5 `object/mod.rs` re-export update

In the `pub use model::{...}` block (`object/mod.rs:68-73`):
- REMOVE `LayoutAlign, LayoutDirection, LayoutSizing` (deleted).
- ADD `LayoutAxis, Lanes, MainAlign, CrossAlign, Align, AxisSizing, Sizing`.
- `Layout` stays.

---

## 3. New ops

### 3.1 `op.rs` variants

Add to `ObjectOp` (`op.rs:48-146`), each `#[serde(rename_all = "camelCase")]`:

```rust
    /// Per-object sizing (Hug/Fill/Fixed per axis). LWW on the "sizing" property.
    #[serde(rename_all = "camelCase")]
    SetSizing {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sizing: Option<Sizing>,
    },

    /// Panel header metadata. Each field optional => absent leaves it unchanged.
    /// `name` uses `FieldEdit` (Set/Clear) so a name can be cleared; bools are a
    /// plain `Option<bool>` (absent = unchanged, present = the new value).
    #[serde(rename_all = "camelCase")]
    SetMeta {
        id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<FieldEdit<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hidden: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locked: Option<bool>,
    },

    /// Extract a geometry's dominant orientation into `transform` and rewrite the
    /// geometry axis-aligned. A geometry-rebake op (NOT zero-rebake).
    #[serde(rename_all = "camelCase")]
    Canonicalize { id: ObjectId },
```

Add imports at `op.rs:8-10`: extend the `use crate::object::model::{...}` with
`FieldEdit` is already local; add `Sizing`. (`FieldEdit` is defined in this file,
`op.rs:19`.)

### 3.2 `op.rs` `kind()` (extend `op.rs:149-169`)

```rust
            ObjectOp::SetSizing { .. } => "set-sizing",
            ObjectOp::SetMeta { .. } => "set-meta",
            ObjectOp::Canonicalize { .. } => "canonicalize",
```

### 3.3 `op.rs` `target_ids()` (extend `op.rs:172-222`)

All three are single-id; add to the `| ObjectOp::Delete { id }` arm group
(`op.rs:181-192`):
```rust
            | ObjectOp::SetSizing { id, .. }
            | ObjectOp::SetMeta { id, .. }
            | ObjectOp::Canonicalize { id }
```

### 3.4 Apply + inverse (`apply.rs`)

Import update (`apply.rs:11-14`): add `Sizing`, `Transform3x3`, and whatever the
affine helper needs; `Geometry` is already imported.

**`SetSizing`** — mirrors `SetLayout` (`apply.rs:169-174`):
```rust
        ObjectOp::SetSizing { id, sizing } => {
            let idx = index_of(scene, &id)?;
            let old = scene.objects[idx].sizing;
            scene.objects[idx].sizing = sizing;
            Ok(ObjectOp::SetSizing { id, sizing: old })
        }
```
Inverse: a `SetSizing` carrying the prior `Option<Sizing>`. Round-trip exact.

**`SetMeta`** — per-field, capturing each prior value into the inverse. Mirrors
`SetStyle`'s per-`FieldEdit` capture (`apply.rs:124-137`):
```rust
        ObjectOp::SetMeta { id, name, hidden, locked } => {
            let idx = index_of(scene, &id)?;
            let inv_name = name.map(|edit| {
                let old = scene.objects[idx].name.clone();
                scene.objects[idx].name = edit.resolve(old.clone());
                FieldEdit::from_option(old)
            });
            let inv_hidden = hidden.map(|v| {
                let old = scene.objects[idx].hidden;
                scene.objects[idx].hidden = v;
                old
            });
            let inv_locked = locked.map(|v| {
                let old = scene.objects[idx].locked;
                scene.objects[idx].locked = v;
                old
            });
            Ok(ObjectOp::SetMeta { id, name: inv_name, hidden: inv_hidden, locked: inv_locked })
        }
```
Inverse: a `SetMeta` whose present fields each carry the prior value (and absent
fields stay absent — an unchanged field is never inverted). Round-trip exact.

**`Canonicalize`** — extract the geometry's dominant angle into `transform`,
rewrite geometry axis-aligned. The INVERSE must restore BOTH the prior geometry
and the prior transform, so it is a `Batch` capturing the pre-state (per the
locked design and `op.rs:121` faithful-inverse convention used by Split/Merge):

```rust
        ObjectOp::Canonicalize { id } => {
            let idx = index_of(scene, &id)?;
            // Capture pre-state for the faithful inverse.
            let old_geometry = scene.objects[idx].geometry.clone();
            let old_transform = scene.objects[idx].transform;

            // Pure geometry-orientation extraction (binding tier helper).
            let (new_transform, new_geometry) =
                crate::object::affine::canonicalize_orientation(&old_transform, &old_geometry);

            // A no-op canonicalize (already axis-aligned / freeform angle 0) yields
            // an empty-Batch no-op inverse so no bogus undo step is pushed (mirrors
            // SetText, apply.rs:144-146).
            if new_transform == old_transform && new_geometry == old_geometry {
                return Ok(ObjectOp::Batch { ops: Vec::new() });
            }

            // Validate the rewritten geometry before committing.
            let mut new_geometry = new_geometry;
            new_geometry.ensure_parsed().map_err(ApplyError::BadGeometry)?;
            validate_geometry(&new_geometry).map_err(apply_error_from_validation)?;

            scene.objects[idx].geometry = new_geometry;
            scene.objects[idx].transform = new_transform;

            // Faithful inverse: restore geometry then transform.
            Ok(ObjectOp::Batch {
                ops: vec![
                    ObjectOp::EditGeometry { id: id.clone(), geometry: old_geometry },
                    ObjectOp::SetTransform { id, transform: old_transform },
                ],
            })
        }
```
Inverse contract check: applying the returned `Batch` restores `old_geometry`
(via `EditGeometry`) and `old_transform` (via `SetTransform`) — exactly the
pre-canonicalize state. `EditGeometry` itself re-validates, satisfying the
one-apply-path rule. The inverse's own inverse (re-inverse for redo) is the
`Batch` of those two ops' inverses reversed, which `apply_inner`'s `Batch` arm
(`apply.rs:229-240`) already produces.

ANCHOR-FOLLOW CONTRACT (chosen: re-home inside apply, like Split/Merge — NOT a
wasm bridge fold): `Canonicalize` rewrites this object's local space while holding
every world point fixed, so a peer anchored onto it would silently desync (its
`at` is in the OLD target-local space). The apply arm captures peer anchors
addressing the canonicalized id BEFORE mutating, re-homes each one's `at` via
`affine::rehome_anchor_local(old_transform, new_transform, at)` (= `new^-1 · old ·
at`, requantized) so the bound world point is preserved, and appends a `SetAnchor`
restoring each peer's pre-state array to the inverse `Batch`. The inverse's
`EditGeometry`+`SetTransform` restore the old local space, and the `SetAnchor`s
restore the old `at`s — round-trip exact. This keeps the contract in the pure
core; no `geometry_follow_ops`/`fold_follower_reprojection` bridge is needed for
canonicalize.

NOTE: do not write a second geometry-rewrite path in apply; the only NEW math is
the pure `canonicalize_orientation` helper in the binding tier (§5). Apply just
calls it and authors ops.

### 3.5 `touched_properties` for LWW (`apply.rs:567-597`)

Add:
```rust
        ObjectOp::SetSizing { id, .. } => vec![(id.clone(), "sizing")],
        ObjectOp::SetMeta { id, name, hidden, locked } => {
            let mut v = Vec::new();
            if name.is_some()   { v.push((id.clone(), "name")); }
            if hidden.is_some() { v.push((id.clone(), "hidden")); }
            if locked.is_some() { v.push((id.clone(), "locked")); }
            v
        }
```
`Canonicalize` is a structural geometry+transform rewrite spanning two properties
atomically — treat it like the other multi-effect structural ops and add it to
the empty-slice arm (`apply.rs:591-596`, alongside `Split`/`Merge`/`Batch`) so the
server orders it by global seq, not per-property. (It is authored as a user click,
not a high-frequency property write, so this is correct.)

### 3.6 Validate (`validate.rs`)

`SetSizing`: no structural invariant beyond the object existing (handled by
`index_of` in apply). For a `Fixed { value }`, value should be `>= 0`
(non-negative quantized extent). Add a small validator and gate it in apply
before mutating:
```rust
// validate.rs
pub fn validate_sizing(sizing: &Sizing) -> Result<(), ValidationError> {
    for ax in [sizing.w, sizing.h] {
        if let AxisSizing::Fixed { value } = ax {
            if value < 0 {
                return Err(ValidationError::NegativeSizing { value });
            }
        }
    }
    Ok(())
}
```
Add `ValidationError::NegativeSizing { value: i32 }` to the enum
(`validate.rs:18-27`) + its `Display` arm (`validate.rs:29-57`) and map it in
`apply_error_from_validation` (`apply.rs:19-30`) to `ApplyError::BadGeometry`
(reuse; no new `ApplyError` variant needed) OR add an `ApplyError::BadSizing`
variant — RECOMMENDED: reuse `BadGeometry(String)` to keep `ApplyError` stable,
mapping the message. Gate in apply's `SetSizing` arm: if `sizing` is `Some`,
`validate_sizing` before assigning.

`SetMeta`: no structural invariant — name is free text, bools are bools. No
validator. (Do not over-validate per the simplicity rule.)

`Layout` reshape: `Lanes::Count { value }` must be `>= 1`. Add a `Layout`
validator gated in `SetLayout`'s apply arm (`apply.rs:169-174`):
```rust
pub fn validate_layout(layout: &Layout) -> Result<(), ValidationError> {
    if let Lanes::Count { value } = layout.lanes {
        if value < 1 {
            return Err(ValidationError::ZeroLanes);
        }
    }
    Ok(())
}
```
Add `ValidationError::ZeroLanes` + `Display` + map to `BadGeometry`. Gate in
`SetLayout`: when `layout` is `Some`, `validate_layout` before assigning.
(`SetLayout` currently has no validation; adding it is in scope because the new
`Lanes::Count` carries an invariant the old `Layout` did not.)

`Canonicalize`: gated by `validate_geometry` on the rewritten geometry (already in
the apply arm, §3.4). No separate validator.

Imports in `validate.rs`: extend `use crate::object::model::{...}`
(`validate.rs:16`) with `AxisSizing, Lanes, Layout, Sizing`.

---

## 4. Golden tests (kernel) — checklist step 6

In `crates/scene-core/tests/object_golden.rs` add tests driving the REAL core
(`apply_object_op`). Each new op needs a falsifiable assertion:

- `set_sizing_round_trips` — apply `SetSizing { Some(Fixed/Fill) }`, assert field
  set; apply inverse, assert back to `None`.
- `set_sizing_rejects_negative_fixed` — `Fixed { value: -1 }` => `ApplyError`,
  scene untouched.
- `set_meta_per_field_inverse` — set `name`+`hidden`, assert both; apply inverse,
  assert prior; assert an absent field is untouched (set only `locked`, leave
  `name` intact).
- `canonicalize_extracts_angle_and_inverse_restores` — build a geometry rotated
  ~30° (nodes), `Canonicalize`, assert `decompose_affine(transform).rotation_rad`
  ≈ 30° and the geometry is axis-aligned (AABB-aligned); apply the `Batch`
  inverse, assert original geometry + transform restored (compare `path_string`
  and `transform`).
- `canonicalize_freeform_is_noop` — a genuinely freeform blob canonicalizes to a
  no-op (rotation 0) with an empty-`Batch` inverse.
- REWRITE `ob53_auto_layout_spaces_children_deterministically`
  (`object_golden.rs:459-494`) and the `layout_solve.rs` unit tests
  (`layout_solve.rs:177-316`) to the new `Layout` shape (see §7).

---

## 5. Pure affine helpers (binding tier: `binding/affine.rs`)

Convention (grounded in `anchor_follow.rs:27-45`): `Transform3x3.m` is row-major
`[[a,c,e],[b,d,f],[0,0,1]]`. The linear part `L = [[a,c],[b,d]]` has column 0 =
`(a,b)` = image of the local x-axis, column 1 = `(c,d)` = image of the local
y-axis; translation = `(e,f)`. All angles in radians, CCW positive in the math
convention; the renderer's y-down is irrelevant to the decomposition (it returns
whatever the matrix encodes).

### 5.1 Public result type + signatures

```rust
/// Affine decomposition result (logical px / radians). Round-trips with
/// `compose_affine` for the common translate+rotate+scale case.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct AffineDecomposition {
    pub translate: (f64, f64),
    pub rotation_rad: f64,
    pub scale: (f64, f64),
    pub skew_rad: f64,
}
```
(Derive `Serialize, Deserialize` too — it crosses the wasm bridge. ts-rs renders
a tuple `(f64,f64)` as `[number, number]`, which the inspector reads.)

```rust
pub fn decompose_affine(t: &Transform3x3) -> AffineDecomposition;
pub fn compose_affine(
    translate: (f64, f64),
    rotation_rad: f64,
    scale: (f64, f64),
    skew_rad: f64,
) -> Transform3x3;
```

### 5.2 Algorithm — `decompose_affine` (QR / Gram-Schmidt of the 2x2 linear part)

Standard 2D affine decomposition into translate · rotate · scale · skew:
1. `translate = (e, f)`.
2. `a,b,c,d` from the linear columns: x-col `(a,b)`, y-col `(c,d)`.
3. `scale_x = hypot(a, b)`. If `scale_x == 0` return identity-ish
   (`rotation 0, scale (0, hypot(c,d)), skew 0`) to avoid div-by-zero.
4. `rotation_rad = atan2(b, a)`.
5. Normalize x-col: `(a/scale_x, b/scale_x)` = unit `(cos, sin)`.
6. `shear = a'·c + b'·d` (dot of unit x-col with y-col).
7. `c' = c - a'·shear`, `d' = d - b'·shear` (remove x-component from y-col).
8. `scale_y = hypot(c', d')`.
9. Fix reflection: if `det = a·d - b·c < 0`, negate `scale_y` (keep rotation as
   the proper-rotation branch). `skew_rad = atan2(shear, scale_y)` when
   `scale_y != 0`, else `0`.

(This is the canonical "QR-like" 2D decomposition; for the common
translate+rotate+uniform-scale case `shear == 0` so `skew_rad == 0` and it
round-trips exactly.)

### 5.3 Algorithm — `compose_affine`

Build `T(translate) · R(rotation_rad) · Sk(skew_rad) · S(scale)` mapped back to
the row-major matrix:
- `R = [[cos, -sin],[sin, cos]]`.
- `Sk = [[1, tan(skew_rad)],[0, 1]]` (skew applied to the y-column).
- `S = [[scale_x, 0],[0, scale_y]]`.
- `L = R · Sk · S`; place into `m[0][0..1]`, `m[1][0..1]`; translation into
  `m[*][2]`; bottom row `[0,0,1]`.

Round-trip target: `compose_affine(decompose_affine(t))  ≈ t` (within 1e-9) for
any translate+rotate+scale matrix (skew 0). Add a test asserting this for a few
sample matrices INCLUDING a reflection-free rotate+nonuniform-scale.

### 5.4 `canonicalize_orientation` (used by the `Canonicalize` op apply)

```rust
/// Returns (new_transform, new_geometry): the geometry's dominant edge angle is
/// extracted into `transform` (composed onto the existing transform) and the
/// geometry is rewritten so its dominant axis is axis-aligned. A freeform shape
/// with no dominant angle returns the inputs unchanged (caller treats as no-op).
pub fn canonicalize_orientation(
    transform: &Transform3x3,
    geometry: &Geometry,
) -> (Transform3x3, Geometry);
```
Algorithm (v1, deliberately simple — refine only if a later phase needs it):
1. Parse/iterate `geometry.subpaths`. If empty after `ensure_parsed`, return
   inputs unchanged.
2. Compute the dominant angle = the angle of the LONGEST edge (segment between
   consecutive nodes, flat across subpaths), measured `atan2(dy, dx)`, folded into
   `[0, π/2)` (mod 90°, since a rect's "dominant axis" is ambiguous by quadrant).
3. If the folded angle is within an epsilon of 0 (already axis-aligned) OR the
   geometry is freeform (no edge meaningfully longer than the next — define
   "dominant" as the longest edge being >= 1.0 px so a blob with all-similar tiny
   edges still has a longest edge; v1 keeps it simple: there is always a longest
   edge, so "freeform reads rotation 0" is realized by the fold-to-0 test, not a
   separate freeform branch), return inputs unchanged → op no-ops.
4. Otherwise: rotate every geometry node by `-angle` about the geometry centroid
   (quantized i32, `.round()` with the same de/quantize discipline as
   `world_to_local_quantized`, `region.rs:144-155`), producing `new_geometry`
   (re-encode via `Geometry::from_subpaths`). Compose `R(angle)` onto the existing
   transform so the world appearance is unchanged: `new_transform = transform ·
   R(angle)` about the same centroid (translate to centroid, rotate, translate
   back, folded into the affine). The world position of every point is preserved
   (canonicalize changes representation, not appearance).

The inspector then honestly reads `rotation_rad ≈ angle` from
`decompose_affine(new_transform)`; the geometry is axis-aligned so its own
selection AABB is tight.

IMPORTANT (zero-rebake nuance): `Canonicalize` DOES rewrite geometry, so it is a
one-shot rebake on explicit user action — acceptable and intended. It is NOT on
the per-frame path.

### 5.5 Re-exports
- `binding/mod.rs`: `pub mod affine;`.
- `object/mod.rs`: `pub use binding::affine;` and `pub use affine::{
  AffineDecomposition, canonicalize_orientation, compose_affine, decompose_affine };`.

---

## 6. Inspector catalog (catalog tier: `catalog/inspector.rs`)

Sibling to `commands.rs` (`catalog/commands.rs`). Static catalog is `Serialize`
ONLY (like `ObjectCommand`, `commands.rs:31-33`); the TS side is hand-declared in
the WF2 shell phase, NOT ts-rs. The dynamic view is `Serialize` only too.

### 6.1 Static catalog types

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectorSection {
    Header,
    Placement,
    Layout,
    Appearance,
    Text,
    Action,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InspectorWidget {
    Text,
    Toggle,
    Badge,
    Button,
    Paint,
    Number { unit: String, min: Option<f64>, max: Option<f64>, step: f64 },
    Segment { options: Vec<String> },
    Lanes,
    Align9,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppliesTo {
    Always,
    FreePlaced,
    FlowChild,
    Container,
    FlowContainer,
    HasText,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorControl {
    pub id: String,
    pub label: String,
    pub section: InspectorSection,
    pub widget: InspectorWidget,
    pub applies_to: AppliesTo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub description: String,
}
```
`object_inspector_catalog() -> Vec<InspectorControl>` and
`object_inspector_catalog_json() -> String` (mirroring
`object_command_catalog`/`_json`, `commands.rs:67-404`).

### 6.2 v1 control inventory

Every `op_kind` below is a real `ObjectOp::kind()` discriminant; every `field`
maps to a real op field or a `decompose_affine` component. `unit`/`min`/`max`/
`step` for `Number` widgets are illustrative; tune in implementation.

| id | section | widget | applies_to | op_kind | field |
|---|---|---|---|---|---|
| `name` | Header | Text | Always | `set-meta` | `name` |
| `visible` | Header | Toggle | Always | `set-meta` | `hidden` (inverted in view) |
| `locked` | Header | Toggle | Always | `set-meta` | `locked` |
| `role` | Header | Badge | Always | — (read-only) | — |
| `x` | Placement | Number (px) | FreePlaced | `set-transform` | `translate.x` |
| `y` | Placement | Number (px) | FreePlaced | `set-transform` | `translate.y` |
| `width` | Placement | Number (px) | FreePlaced | `set-transform` | `scale.x` |
| `height` | Placement | Number (px) | FreePlaced | `set-transform` | `scale.y` |
| `rotation` | Placement | Number (deg) | FreePlaced | `set-transform` | `rotation` |
| `sizing-w` | Placement | Segment [Hug,Fill,Fixed] | FlowChild | `set-sizing` | `w` |
| `sizing-h` | Placement | Segment [Hug,Fill,Fixed] | FlowChild | `set-sizing` | `h` |
| `rotation-flow` | Placement | Number (deg) | FlowChild | `set-transform` | `rotation` |
| `layout-mode` | Layout | Segment [Free,Flow] | Container | `set-layout` | `mode` |
| `axis` | Layout | Segment [Horizontal,Vertical] | FlowContainer | `set-layout` | `axis` |
| `lanes` | Layout | Lanes | FlowContainer | `set-layout` | `lanes` |
| `spacing` | Layout | Number (px) | FlowContainer | `set-layout` | `spacing` |
| `align` | Layout | Align9 | FlowContainer | `set-layout` | `align` |
| `clip` | Layout | Toggle | FlowContainer | `set-clip` | `clip` |
| `fill` | Appearance | Paint | Always | `set-style` | `fill` |
| `stroke` | Appearance | Paint | Always | `set-style` | `stroke` |
| `stroke-width` | Appearance | Number (px) | Always | `set-style` | `stroke.width` |
| `font` | Text | Segment (families) | HasText | `set-text` | `font` |
| `font-size` | Text | Number (px) | HasText | `set-text` | `size` |
| `font-weight` | Text | Segment [Regular,Bold] | HasText | `set-text` | `bold` |
| `text-align` | Text | Segment [Start,Center,End,Justify] | HasText | `set-text` | `align` |
| `text-color` | Text | Paint | HasText | `set-text` | `color` |
| `canonicalize` | Action | Button | Always | `canonicalize` | — |

Notes:
- `layout-mode`'s `field: "mode"` is a synthetic field the view encodes as
  `"free"`/`"flow"`; the shell maps a Flow pick to `SetLayout { layout:
  Some(default Layout) }` and a Free pick to `SetLayout { layout: None }`. The
  op_kind is the real `set-layout`.
- `visible` toggles `hidden` (the stored field) but presents inverted in the
  value-read (see 6.4); the `field` is `hidden`.
- `width`/`height`/`x`/`y`/`rotation` ride `set-transform` via `compose_affine`
  in the shell from the inspector-edited component (the shell does NO matrix math
  — it reads components via `object_decompose_transform` and writes back via
  `object_compose_transform`, §8).
- Appendix: opacity + corner-radius DEFERRED (out of scope); per-child align-self
  override out of scope; connector/anchor UI out of scope.

### 6.3 Role resolution (pure, catalog tier)

```rust
pub struct InspectorRole {
    pub placement: Placement,   // Free | FlowChild  (serde "free" | "flow-child")
    pub container: bool,
    pub flow_container: bool,
    pub has_text: bool,
}
```
Resolution for an object `obj` in `scene`:
- `placement` = `obj.parent` resolves to an object whose `layout.is_some()`
  (a Flow container) ? `flow-child` : `free`. (Parent lookup via
  `scene.get(parent_id)`; `layout` field from §2.1.)
- `container` = `has_children(scene, obj.id)` (reuse
  `grouping::has_children`, `grouping.rs:20-22`).
- `flow_container` = `container && obj.layout.is_some()`.
- `has_text` = `obj.text.is_some()` (`model.rs:569`).

An `AppliesTo` matches a role iff:
- `Always` => always.
- `FreePlaced` => `placement == Free`.
- `FlowChild` => `placement == FlowChild`.
- `Container` => `container`.
- `FlowContainer` => `flow_container`.
- `HasText` => `has_text`.

### 6.4 Dynamic view + value reading

```rust
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorView {
    pub role: InspectorRoleWire,            // { placement, container, flowContainer, hasText }
    pub sections: Vec<InspectorSectionView>, // [{ section, controls: [InspectorControlValue] }]
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorControlValue {
    pub id: String,
    pub label: String,
    pub widget: InspectorWidget,
    pub value: serde_json::Value,   // null when not-applicable/unset
    pub mixed: bool,                // multi-select divergence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

pub fn inspector_view(scene: &ObjectScene, selection: &ObjectSelection) -> InspectorView;
```
Selection resolution (reuse `ObjectSelection`, `model.rs:638-642`):
- `Canvas` => empty role (`placement: free`, all bools false), no controls (or a
  canvas-level section if a later phase wants it; v1: empty sections).
- `Object { id }` => single-object role + values.
- `Multi { ids }` => intersection of applicable controls across all selected
  objects (a control shows only if it applies to EVERY selected object's role);
  `value` = the common value or `null` with `mixed: true` when they diverge.

Per-control `value` reading (single object):
- `name` => `obj.name` (string or null).
- `visible` => `!obj.hidden` (inverted).
- `locked` => `obj.locked`.
- `role` => the role string (e.g. `"Free placed"`, `"Flow child"`, `"Flow
  container"`) — display-only.
- `x`/`y` => `decompose_affine(obj.transform).translate.{0,1}`.
- `width`/`height` => `decompose_affine(obj.transform).scale.{0,1}` (px extent =
  scale times local geometry extent; v1: report the decomposition scale directly,
  the shell multiplies by the local AABB if it wants px — KEEP THE CORE READING
  THE DECOMPOSITION, do not re-derive bounds here).
- `rotation`/`rotation-flow` => `decompose_affine(obj.transform).rotation_rad`
  converted to degrees.
- `sizing-w`/`sizing-h` => `obj.sizing.map(|s| s.w/s.h)` else `Hug` default
  (serialize the `AxisSizing` tag, e.g. `"hug"`/`"fill"`/`"fixed"` + value).
- `layout-mode` => `if obj.layout.is_some() { "flow" } else { "free" }`.
- `axis`/`lanes`/`spacing`/`align` => from `obj.layout` (null when Free).
- `clip` => `obj.clip` (default false).
- `fill`/`stroke` => `obj.fill`/`obj.stroke` (the `Paint`, or null).
- `stroke-width` => `obj.stroke.map(|s| s.width)`.
- `font`/`font-size`/`font-weight`/`text-align`/`text-color` => from
  `obj.text` runs[0] / `obj.text.align` (null when no text).
- `canonicalize` => no value (button).

PURE: no IO/time/rng. Reads rotation/scale ONLY via `decompose_affine` (the single
source of truth per the locked design).

### 6.5 commands.rs additions (`commands.rs:67-398`)

Add a `canonicalize` command to `object_command_catalog()` (Path or a new use of
the existing `Edit`/`Arrange`; RECOMMENDED category `Path` since it reshapes
geometry):
```rust
ObjectCommand::new(
    "canonicalize",
    "Straighten / 방향 정규화",
    Path,
    None,
    "Extract the dominant orientation into the transform and re-axis-align the geometry.",
    Some("canonicalize"),
),
```
Add it to `requested_commands_all_present` (`commands.rs:445-491`) and ensure
`op_kinds_are_real_object_op_discriminants` (`commands.rs:493-606`) gets a probe
arm for the new `Canonicalize`/`SetSizing`/`SetMeta` ops (extend the `known` set
literal so the test stays in lockstep).

The `layout-mode` toggle maps to the real `set-layout` op (no command needed; it
is inspector-only). If a top-level shortcut is wanted later, add a
`toggle-layout-mode` command with `op_kind: Some("set-layout")`.

### 6.6 Re-exports
- `catalog/mod.rs`: `pub mod inspector;`.
- `object/mod.rs`: `pub use catalog::inspector;` and `pub use inspector::{
  object_inspector_catalog, object_inspector_catalog_json, inspector_view,
  InspectorControl, InspectorSection, InspectorWidget, AppliesTo, InspectorView,
  ... }`.

---

## 7. group_ops Flow-default + axis inference (`binding/grouping.rs`)

CURRENT (`grouping.rs:62-113`): `group_ops` mints a frame with NO `layout`
(implicitly Free) and `clip: Some(false)`.

CHANGE: default new frames to **Flow**, axis inferred from the children's spatial
spread:
- After computing the union world-AABB (`grouping.rs:76-88`), compute the
  children's bounding spread: `span_w = max_x - min_x`, `span_h = max_y - min_y`.
- `axis = if span_w >= span_h { LayoutAxis::Horizontal } else {
  LayoutAxis::Vertical }` (wider horizontal spread => Horizontal/Row; taller =>
  Vertical).
- Set `frame.layout = Some(Layout { axis, lanes: Lanes::Count { value: 1 },
  spacing: <default>, align: Align { main: MainAlign::Start, cross:
  CrossAlign::Start } })` where `<default>` is a small quantized inset (e.g. `0`
  for v1 to preserve existing golden positions, OR a sensible default like
  `GEOMETRY_QUANTUM_PER_PX * 8` — DECISION: use `0` in v1 so the existing
  `group_ops_*` golden tests' frame geometry/transform assertions
  (`grouping.rs:285-330`) do not shift; spacing is adjustable in the inspector).
- Keep `clip: Some(false)` and the rect geometry as-is.

Imports (`grouping.rs:6-8`): add `Align, Layout, LayoutAxis, Lanes, MainAlign,
CrossAlign` to the `use crate::object::model::{...}`.

TESTS to update/add (`grouping.rs:285-337`):
- Add `group_ops_defaults_to_flow_with_inferred_axis`: build two members spread
  WIDER horizontally => assert `frame.layout == Some(Layout { axis: Horizontal,
  .. })`; build a taller spread => `Vertical`.
- The existing `group_ops_frame_is_sized_and_placed_to_the_union_world_aabb`
  (`grouping.rs:285-300`) still passes IF `spacing: 0` keeps the frame geometry
  identical (it does — layout is metadata, geometry is the union rect).

NOTE: the layout SOLVE (`layout_solve.rs`) is a separate concern; grouping only
sets the `Layout` metadata. The solve must be rewritten to the new `Layout` shape
(§ below) but grouping does not call it.

### 7.1 layout_solve.rs rewrite (required by the type reshape)

`solve_layout` (`layout_solve.rs:70-138`) references `layout.direction`,
`layout.padding`, `layout.gap`, `layout.align`, `LayoutAlign`, `LayoutDirection`,
`LayoutSizing`. Rewrite to the new shape:
- `dir`: `layout.axis` (`Horizontal` => main x, `Vertical` => main y) instead of
  `direction` Row/Column.
- `padding_px` AND `gap_px` both come from the single `layout.spacing`
  (`to_px(layout.spacing)`) — spacing IS both the edge inset and inter-child gap.
- cross-align: `layout.align.cross` (`CrossAlign::Start|Center|End|Stretch`);
  `Stretch` still falls back to `Start` (a transform cannot resize; same caveat as
  `layout_solve.rs:115-122`).
- main-align: `layout.align.main` — v1 may keep packing from `spacing` start and
  treat `Center/End/SpaceBetween` as a follow-up (DECISION: v1 honors only
  `Start` for main, with a `// TODO`-free comment noting Center/End/SpaceBetween
  need the group box this slice does not carry — mirror the existing Hug-only
  note `layout_solve.rs:12-15`). Keep it honest: do not silently mis-place.
- `lanes`: `Lanes::Count { value: 1 }` => single track (current behavior);
  `Count { N>1 }`/`Fill` (grid/wrap) is NOT solved in v1 — fall back to single
  track packing with a note, same honesty as the sizing fallback.
- Imports (`layout_solve.rs:17-19`): replace `LayoutAlign, LayoutDirection` with
  `CrossAlign, LayoutAxis`.
- Rewrite the 6 unit tests (`layout_solve.rs:177-316`) and the `ob53_*` golden
  (`object_golden.rs:459-494`) to construct the new `Layout`.

---

## 8. wasm exports (`wasm_api.rs`, thin)

Mirror the catalog/decompose surface. Add (after `object_command_catalog`,
`wasm_api.rs:143-151`):

```rust
#[wasm_bindgen]
pub fn object_inspector_catalog() -> String {
    object_inspector_catalog_json()
}

/// `scene_json` is an `ObjectScene`; `selection_json` is an `ObjectSelection`.
#[wasm_bindgen]
pub fn object_inspector_view(scene_json: &str, selection_json: &str) -> String {
    let mut scene: ObjectScene = match parse("scene", scene_json) { Ok(v) => v, Err(e) => return e };
    if let Err(e) = scene.ensure_parsed() {
        return error_json(&format!("scene geometry parse failed: {e}"));
    }
    let selection: ObjectSelection = match parse("selection", selection_json) { Ok(v) => v, Err(e) => return e };
    ok_json(&inspector_view(&scene, &selection))
}

/// `matrix_json` is the bare `[[f64;3];3]` (the transparent Transform3x3 wire form).
#[wasm_bindgen]
pub fn object_decompose_transform(matrix_json: &str) -> String {
    let t: Transform3x3 = match parse("matrix", matrix_json) { Ok(v) => v, Err(e) => return e };
    ok_json(&decompose_affine(&t))
}

/// `params_json` is `{ translate:[x,y], rotationRad, scale:[x,y], skewRad }`.
#[wasm_bindgen]
pub fn object_compose_transform(params_json: &str) -> String {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params { translate: [f64; 2], rotation_rad: f64, scale: [f64; 2], skew_rad: f64 }
    let p: Params = match parse("params", params_json) { Ok(v) => v, Err(e) => return e };
    let t = compose_affine((p.translate[0], p.translate[1]), p.rotation_rad, (p.scale[0], p.scale[1]), p.skew_rad);
    ok_json(&t)
}
```
Imports (`wasm_api.rs:53-87`): add
`use crate::object::inspector::{inspector_view, object_inspector_catalog_json};`
and `use crate::object::affine::{compose_affine, decompose_affine};`. `Transform3x3`
+ `ObjectSelection` are already imported (`wasm_api.rs:53`).

These are THIN: parse → call pure fn → serialize. No logic. (Verify by inspection
against the existing `object_command_catalog` bridge, `wasm_api.rs:143-146`.)

---

## 9. Complete call-site list the Model phase MUST update

Grepped across the WHOLE workspace (`grep -rn crates/`). The only scene-`Layout`
family touch points (excluding unrelated `std::alloc::Layout`, wgpu
`BindGroupLayout`, and `text_layout`):

**Type definitions (REWRITE in place):**
1. `crates/scene-core/src/object/kernel/model.rs:463-470` — `LayoutDirection`
   enum (DELETE).
2. `crates/scene-core/src/object/kernel/model.rs:472-481` — `LayoutAlign` enum
   (DELETE).
3. `crates/scene-core/src/object/kernel/model.rs:483-491` — `LayoutSizing` enum
   (DELETE).
4. `crates/scene-core/src/object/kernel/model.rs:493-505` — `Layout` struct
   (RESHAPE).
5. `crates/scene-core/src/object/kernel/model.rs:573-574` — `Object.layout`
   field (UNCHANGED type `Option<Layout>`; add `sizing`/`name`/`hidden`/`locked`
   nearby).
6. `crates/scene-core/src/object/kernel/model.rs:608` — `Object::new` `layout:
   None` (add `sizing: None, name: None, hidden: false, locked: false`).

**Imports / op payload:**
7. `crates/scene-core/src/object/kernel/op.rs:9` — `use ...{ ..., Layout, ... }`
   (keep; add `Sizing`).
8. `crates/scene-core/src/object/kernel/op.rs:90` — `SetLayout { layout:
   Option<Layout> }` (UNCHANGED shape).

**Apply:**
9. `crates/scene-core/src/object/kernel/apply.rs:12` — `use ...{ ..., Layout, ...
   }` (keep; add `Sizing`).
10. `crates/scene-core/src/object/kernel/apply.rs:171-173` — `SetLayout` apply
    arm (UNCHANGED logic; add `validate_layout` gate per §3.6).

**Layout solve (REWRITE — §7.1):**
11. `crates/scene-core/src/object/binding/layout_solve.rs:17-19` — imports
    (`LayoutAlign, LayoutDirection`).
12. `crates/scene-core/src/object/binding/layout_solve.rs:51-63` — `main_extent`/
    `cross_extent` (`LayoutDirection::Row/Column`).
13. `crates/scene-core/src/object/binding/layout_solve.rs:100-131` — solve body
    (`layout.direction`, `layout.padding`, `layout.gap`, `layout.align`,
    `LayoutAlign::*`, `LayoutDirection::*`).
14. `crates/scene-core/src/object/binding/layout_solve.rs:143` — test import
    (`Layout, LayoutSizing`).
15. `crates/scene-core/src/object/binding/layout_solve.rs:177-316` — 6 unit tests
    constructing the old `Layout`.

**Grouping (FLOW DEFAULT — §7):**
16. `crates/scene-core/src/object/binding/grouping.rs:6-8` — imports (add layout
    types).
17. `crates/scene-core/src/object/binding/grouping.rs:91-99` — frame construction
    (add `frame.layout = Some(...)`).

**Facade re-exports:**
18. `crates/scene-core/src/object/mod.rs:68-73` — `pub use model::{ ...,
    LayoutAlign, LayoutDirection, LayoutSizing, ... }` (remove the three deleted,
    add the seven new types + `Sizing`/`AxisSizing`).

**Tests (golden):**
19. `crates/scene-core/tests/object_golden.rs:459-494` — `ob53_auto_layout_*`
    constructs the old `Layout` (REWRITE).

**Commands catalog (op-kind probe + new command):**
20. `crates/scene-core/src/object/catalog/commands.rs:493-606` —
    `op_kinds_are_real_object_op_discriminants` `known` set (extend with the three
    new ops so the lockstep test compiles).

NO server / storage-core / client-runtime / coordination / renderer call sites
construct or match scene `Layout`/`LayoutDirection`/`LayoutAlign`/`LayoutSizing`.
(`client-runtime/tests/common/mod.rs:37` sets `layout: None` — UNCHANGED, no type
edit needed.) The only `Layout` token hits in those crates are
`std::alloc::Layout` (`storage-core/src/lib.rs:214,224,233`) and GPU/text layout
in `renderer-wgpu`/`renderer-core` — UNRELATED, do not touch.

---

## 10. Per-op 6-step checklist coverage (verification that nothing is skipped)

| Op | (1) op.rs variant + kind + target_ids | (2) apply + inverse | (3) validate | (4) wire authoring/binding/catalog | (5) ts:gen + commit generated | (6) golden test |
|---|---|---|---|---|---|---|
| `SetSizing` | §3.1/3.2/3.3 | §3.4 (mirror SetLayout) | §3.6 (`validate_sizing`) | inspector `sizing-w`/`sizing-h` controls §6.2 | regen `object-wire.ts` | §4 `set_sizing_*` |
| `SetMeta` | §3.1/3.2/3.3 | §3.4 (per-field FieldEdit) | §3.6 (none) | inspector `name`/`visible`/`locked` §6.2 | regen | §4 `set_meta_*` |
| `Canonicalize` | §3.1/3.2/3.3 | §3.4 (Batch inverse) | §3.6 (`validate_geometry`) | command + inspector `canonicalize` button §6.2/6.5 | regen | §4 `canonicalize_*` |

`SetLayout` is NOT a new op (shape unchanged) but its inner `Layout` reshaped, so
it needs: ts:gen regen (step 5) + the rewritten layout_solve/grouping/golden tests
(steps 4/6) + the new `validate_layout` gate (step 3).

ts:gen: after the model/op edits, run `cd platforms/web && npm run types:gen` and
commit `platforms/web/shared/generated/object-wire.ts`. The new `AffineDecomposition`
also exports there (it derives ts-rs). The inspector catalog/view types do NOT
ts-rs export (Serialize-only; hand-declared in the shell phase). Verify with
`npm run types:check`.

---

## 11. Internal-consistency verification (done)

- Every inspector control's `op_kind` is a real `ObjectOp::kind()` string:
  `set-meta`, `set-transform`, `set-sizing`, `set-layout`, `set-clip`,
  `set-style`, `set-text`, `canonicalize` — all present in §3.2 or the existing
  `op.rs:149-169`.
- Every control `field` maps to a real op field (`name`/`hidden`/`locked` on
  `SetMeta`; `w`/`h` on `SetSizing`; `axis`/`lanes`/`spacing`/`align` on
  `Layout`; `clip` on `SetClip`; `fill`/`stroke`/`stroke.width` on `SetStyle`;
  `font`/`size`/`bold`/`align`/`color` on `Text`) OR a `decompose_affine`
  component (`translate.x/y`, `scale.x/y`, `rotation`) for `set-transform`. The
  synthetic `layout-mode` `mode` field is documented as a view-encoded
  free/flow selector that lowers to `set-layout`.
- Every new op has all 6 checklist steps planned (§10 table).
- Rotation/scale are read ONLY through `decompose_affine` (locked design honored:
  single source of truth, freeform reads rotation 0).
- Position channel is structurally single: Free => child `transform`; Flow =>
  parent `Layout` (no per-child override field exists). Confirmed: no per-child
  absolute-override field is added.

## 12. Verification commands (run after each phase)

- `scripts/renderer-toolchain.sh cargo test -p shape_scene_core`
- `scripts/renderer-toolchain.sh cargo build --workspace`
- `make check-wasm`
- `cd platforms/web && npm run types:gen && npm run types:check`
- `cd platforms/web && npm run scene:wasm:build`
- `make test-rust`
