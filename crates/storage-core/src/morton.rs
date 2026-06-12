//! Morton (Z-order) keys for region windowing.
//!
//! A KV store has no SQL `WHERE bbox && window`. Interleaving a quantized 2D
//! point's bits into a `u64` and keying by `canvas_id` prefix + big-endian
//! Morton code makes byte order equal Z-order, so a region window becomes a
//! **single ordered range scan** plus an exact bbox-overlap refilter.
//!
//! Pointer-width-agnostic: codes are `u64`, axis inputs `u32`, never `usize`.
//! World `f64` maps to `u32` via a fixed offset so negative coordinates encode
//! correctly and the index stays stable across runs.

/// World-coordinate origin offset. World `f64` (rounded) is shifted by `2^31`
/// into `u32` space, giving a representable range of roughly `[-2^31, 2^31)`
/// around the origin while keeping negative coordinates in Z-order.
const WORLD_OFFSET: i64 = 1 << 31;

/// Map a world `f64` coordinate to a `u32` axis value (round-to-nearest, clamped
/// into the representable range). The clamp makes the subsequent narrowing cast
/// lossless for every input, including NaN/±inf.
#[allow(
    clippy::cast_possible_truncation,
    reason = "value is clamped into [0, u32::MAX] before the cast, so truncation cannot occur"
)]
pub fn world_to_axis(v: f64) -> u32 {
    if v.is_nan() {
        return u32::MAX / 2; // NaN -> origin-ish; never panics
    }
    let shifted = v.round() + WORLD_OFFSET as f64;
    let clamped = shifted.clamp(0.0, u32::MAX as f64);
    clamped as u32
}

/// Spread the low 32 bits of `v` so each bit `i` lands at position `2*i`.
fn spread(v: u32) -> u64 {
    let mut n = u64::from(v);
    n = (n | (n << 16)) & 0x0000_FFFF_0000_FFFF;
    n = (n | (n << 8)) & 0x00FF_00FF_00FF_00FF;
    n = (n | (n << 4)) & 0x0F0F_0F0F_0F0F_0F0F;
    n = (n | (n << 2)) & 0x3333_3333_3333_3333;
    n = (n | (n << 1)) & 0x5555_5555_5555_5555;
    n
}

/// Inverse of [`spread`]: gather the even bits of `n` back into a `u32`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "masked to the low 32 bits before the cast"
)]
fn compact(n: u64) -> u32 {
    let mut n = n & 0x5555_5555_5555_5555;
    n = (n | (n >> 1)) & 0x3333_3333_3333_3333;
    n = (n | (n >> 2)) & 0x0F0F_0F0F_0F0F_0F0F;
    n = (n | (n >> 4)) & 0x00FF_00FF_00FF_00FF;
    n = (n | (n >> 8)) & 0x0000_FFFF_0000_FFFF;
    n = (n | (n >> 16)) & 0x0000_0000_FFFF_FFFF;
    n as u32
}

/// Interleave `(x, y)` into a `u64` Morton code (x in even bits, y in odd bits).
pub fn morton_encode(x: u32, y: u32) -> u64 {
    spread(x) | (spread(y) << 1)
}

/// Recover `(x, y)` from a Morton code.
pub fn morton_decode(code: u64) -> (u32, u32) {
    (compact(code), compact(code >> 1))
}

/// Morton code of a world point (the bbox center is the usual choice).
pub fn morton_of_world(x: f64, y: f64) -> u64 {
    morton_encode(world_to_axis(x), world_to_axis(y))
}

/// The inclusive `[lo, hi]` Morton range that bounds a world-space AABB. The
/// range over-covers the box (the Z-curve wanders outside it), so a region scan
/// must refilter each candidate with the exact bbox test. `lo`/`hi` are the
/// Morton codes of the `(min_x,min_y)` and `(max_x,max_y)` corners.
pub fn morton_range(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> (u64, u64) {
    let lo = morton_of_world(min_x, min_y);
    let hi = morton_of_world(max_x, max_y);
    (lo.min(hi), lo.max(hi))
}

/// The big-endian keyspace row for a record: `canvas_id` + `0x00` separator +
/// 8 Morton bytes + `object_id`. Byte order == (canvas, Morton, object_id)
/// order, so an ordered range scan yields exactly the Z-order window; the
/// `object_id` tail keeps the key UNIQUE so two records in the same Morton cell
/// get distinct rows instead of clobbering each other.
pub fn region_row_key(canvas_id: &str, morton: u64, object_id: &str) -> Vec<u8> {
    let mut key = region_scan_start(canvas_id, morton);
    key.extend_from_slice(object_id.as_bytes());
    key
}

/// The inclusive lower bound of a Morton cell's rows: `canvas\0` + 8 Morton bytes,
/// with no `object_id` tail, so it sorts at or before every [`region_row_key`] in
/// that cell. Doubles as a region-scan window's start key.
pub fn region_scan_start(canvas_id: &str, morton: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(canvas_id.len() + 1 + 8 + 16);
    key.extend_from_slice(canvas_id.as_bytes());
    key.push(0x00);
    key.extend_from_slice(&morton.to_be_bytes());
    key
}

/// The EXCLUSIVE upper bound ending a region scan after every row in cell
/// `hi_morton`: the first key of the next cell, `canvas\0 + (hi_morton + 1)`.
/// At `u64::MAX` there is no next cell, so bump the `0x00` separator to `0x01` —
/// the first key past the whole canvas's Morton space.
pub fn region_scan_end_excl(canvas_id: &str, hi_morton: u64) -> Vec<u8> {
    match hi_morton.checked_add(1) {
        Some(next) => region_scan_start(canvas_id, next),
        None => {
            let mut key = Vec::with_capacity(canvas_id.len() + 1);
            key.extend_from_slice(canvas_id.as_bytes());
            key.push(0x01);
            key
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morton_round_trips() {
        for &(x, y) in &[(0u32, 0u32), (1, 0), (0, 1), (12345, 67890), (u32::MAX, u32::MAX)] {
            let code = morton_encode(x, y);
            assert_eq!(morton_decode(code), (x, y));
        }
    }

    #[test]
    fn world_axis_preserves_order_across_origin() {
        assert!(world_to_axis(-100.0) < world_to_axis(0.0));
        assert!(world_to_axis(0.0) < world_to_axis(100.0));
    }

    #[test]
    fn world_axis_clamps_extremes() {
        assert_eq!(world_to_axis(f64::INFINITY), u32::MAX);
        assert_eq!(world_to_axis(f64::NEG_INFINITY), 0);
    }

    #[test]
    fn morton_range_is_ordered_and_contains_corners() {
        let (lo, hi) = morton_range(-10.0, -10.0, 10.0, 10.0);
        assert!(lo <= hi);
        let center = morton_of_world(0.0, 0.0);
        assert!(lo <= center && center <= hi);
    }

    #[test]
    fn region_row_key_orders_by_canvas_then_morton_then_id() {
        let a = region_row_key("canvas-a", 5, "obj");
        let b = region_row_key("canvas-a", 9, "obj");
        let c = region_row_key("canvas-b", 1, "obj");
        assert!(a < b, "same canvas orders by morton");
        assert!(b < c, "canvas prefix dominates");

        let o1 = region_row_key("canvas-a", 5, "id-1");
        let o2 = region_row_key("canvas-a", 5, "id-2");
        assert_ne!(o1, o2, "same cell, distinct ids -> distinct rows");
        assert!(o1 < o2, "rows in a cell order by object id");
        let start = region_scan_start("canvas-a", 5);
        let end = region_scan_end_excl("canvas-a", 5);
        assert!(start <= o1 && o1 < end && o2 < end, "scan brackets the cell");

        let max_row = region_row_key("canvas-a", u64::MAX, "z");
        let max_end = region_scan_end_excl("canvas-a", u64::MAX);
        assert!(max_row < max_end, "u64::MAX cell has a valid exclusive end");
    }
}
