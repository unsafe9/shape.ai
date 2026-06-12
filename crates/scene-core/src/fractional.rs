//! Fractional indexing for z-order. Faithful line-for-line port of
//! `rocicorp/fractional-indexing` (`src/index.js`, CC0) on the fixed base-62
//! alphabet, byte-identical to that reference — the wasm client and native
//! server both generate keys and must agree, and golden vectors must stay stable.
//!
//! Invariant: keys order correctly under plain byte / `str` comparison —
//! [`generate_key_between`]`(a, b)` returns `k` with `a < k < b` lexically. Sort
//! with ordinary `str` `Ord` ([`cmp_keys`]); no custom comparator, no parsing.
//!
//! Encoding: `integer_part ++ fractional_part` over base-62 digits ordered
//! `0-9`, `A-Z`, `a-z` (a digit's index == its value == its sort rank). The
//! integer head byte encodes sign and width — `'a'..='z'` positive (length
//! `head - 'a' + 2`), `'A'..='Z'` negative (length `'Z' - head + 2`); canonical
//! zero is `"a0"`. The fractional part never emits a trailing `'0'`.
//!
//! Pure: no randomness, time, IO, or allocation beyond the returned `String`s.

use std::cmp::Ordering;

const BASE_62_DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

const ZERO: u8 = b'0';
const MAX_DIGIT: u8 = b'z';

/// The sentinel smallest integer part, `"A" + "0".repeat(26)`, rejected as a key.
fn smallest_integer() -> String {
    let mut s = String::with_capacity(27);
    s.push('A');
    for _ in 0..26 {
        s.push('0');
    }
    s
}

/// Plain byte order — exposed so call sites never reach for a custom comparator
/// and diverge from the storage/sort contract.
#[inline]
pub fn cmp_keys(a: &str, b: &str) -> Ordering {
    a.cmp(b)
}

fn digit_index(c: u8) -> Result<usize, String> {
    match c {
        b'0'..=b'9' => Ok((c - b'0') as usize),
        b'A'..=b'Z' => Ok((c - b'A') as usize + 10),
        b'a'..=b'z' => Ok((c - b'a') as usize + 36),
        _ => Err(format!("invalid order-key digit: {:?}", c as char)),
    }
}

fn get_integer_length(head: u8) -> Result<usize, String> {
    match head {
        b'a'..=b'z' => Ok((head - b'a') as usize + 2),
        b'A'..=b'Z' => Ok((b'Z' - head) as usize + 2),
        _ => Err(format!("invalid order key head: {:?}", head as char)),
    }
}

fn get_integer_part(key: &str) -> Result<&str, String> {
    if key.is_empty() {
        return Err("invalid order key: empty".to_string());
    }
    let len = get_integer_length(key.as_bytes()[0])?;
    if len > key.len() {
        return Err(format!("invalid order key: {key:?}"));
    }
    Ok(&key[..len])
}

fn validate_integer(int: &str) -> Result<(), String> {
    if int.is_empty() {
        return Err("invalid integer part of order key: empty".to_string());
    }
    if int.len() != get_integer_length(int.as_bytes()[0])? {
        return Err(format!("invalid integer part of order key: {int:?}"));
    }
    Ok(())
}

/// Rejects the smallest-integer sentinel and a trailing fractional zero.
fn validate_order_key(key: &str) -> Result<(), String> {
    if key == smallest_integer() {
        return Err(format!("invalid order key: {key:?}"));
    }
    let i = get_integer_part(key)?;
    let f = &key[i.len()..];
    for &c in &i.as_bytes()[1..] {
        digit_index(c)?;
    }
    for &c in f.as_bytes() {
        digit_index(c)?;
    }
    if let Some(&last) = f.as_bytes().last() {
        if last == ZERO {
            return Err(format!("invalid order key: {key:?}"));
        }
    }
    Ok(())
}

/// Midpoint of two fractional digit strings. `a` may be empty; `b` is `None`
/// (open upper end) or a non-empty string with `a < b`. Neither may end in `'0'`.
fn midpoint(a: &str, b: Option<&str>) -> Result<String, String> {
    if let Some(b) = b {
        if a >= b {
            return Err(format!("{a:?} >= {b:?}"));
        }
    }
    if a.as_bytes().last() == Some(&ZERO)
        || b.map(|b| b.as_bytes().last() == Some(&ZERO)).unwrap_or(false)
    {
        return Err("trailing zero".to_string());
    }

    if let Some(b) = b {
        // Strip the longest common prefix, zero-padding `a` as we go.
        let ab = a.as_bytes();
        let bb = b.as_bytes();
        let mut n = 0;
        while n < bb.len() && (if n < ab.len() { ab[n] } else { ZERO }) == bb[n] {
            n += 1;
        }
        if n > 0 {
            let rest_a = if n < a.len() { &a[n..] } else { "" };
            let inner = midpoint(rest_a, Some(&b[n..]))?;
            let mut out = String::with_capacity(n + inner.len());
            out.push_str(&b[..n]);
            out.push_str(&inner);
            return Ok(out);
        }
    }

    let digit_a = if !a.is_empty() { digit_index(a.as_bytes()[0])? } else { 0 };
    let digit_b = match b {
        Some(b) => digit_index(b.as_bytes()[0])?,
        None => BASE_62_DIGITS.len(),
    };

    if digit_b - digit_a > 1 {
        // Round(0.5 * (a + b)). For integers, this is floor((a + b + 1) / 2).
        let mid_digit = (digit_a + digit_b + 1) / 2;
        return Ok((BASE_62_DIGITS[mid_digit] as char).to_string());
    }

    match b {
        Some(b) if b.len() > 1 => Ok(b[..1].to_string()),
        _ => {
            // `b` None or single digit: descend into `a`'s tail under its first digit.
            let first = BASE_62_DIGITS[digit_a];
            let rest_a = if a.len() > 1 { &a[1..] } else { "" };
            let inner = midpoint(rest_a, None)?;
            let mut out = String::with_capacity(1 + inner.len());
            out.push(first as char);
            out.push_str(&inner);
            Ok(out)
        }
    }
}

/// Next integer part, or `None` on overflow.
fn increment_integer(x: &str) -> Result<Option<String>, String> {
    validate_integer(x)?;
    let head = x.as_bytes()[0];
    let mut digs: Vec<u8> = x.as_bytes()[1..].to_vec();

    let mut carry = true;
    let mut i = digs.len();
    while carry && i > 0 {
        i -= 1;
        let d = digit_index(digs[i])? + 1;
        if d == BASE_62_DIGITS.len() {
            digs[i] = ZERO;
        } else {
            digs[i] = BASE_62_DIGITS[d];
            carry = false;
        }
    }

    if carry {
        if head == b'Z' {
            return Ok(Some(format!("a{}", ZERO as char)));
        }
        if head == MAX_DIGIT {
            return Ok(None);
        }
        let h = head + 1;
        if h > b'a' {
            digs.push(ZERO);
        } else {
            digs.pop();
        }
        Ok(Some(make_int(h, &digs)))
    } else {
        Ok(Some(make_int(head, &digs)))
    }
}

/// Previous integer part, or `None` on underflow.
fn decrement_integer(x: &str) -> Result<Option<String>, String> {
    validate_integer(x)?;
    let head = x.as_bytes()[0];
    let mut digs: Vec<u8> = x.as_bytes()[1..].to_vec();

    let mut borrow = true;
    let mut i = digs.len();
    while borrow && i > 0 {
        i -= 1;
        let dv = digit_index(digs[i])?;
        if dv == 0 {
            digs[i] = MAX_DIGIT;
        } else {
            digs[i] = BASE_62_DIGITS[dv - 1];
            borrow = false;
        }
    }

    if borrow {
        if head == b'a' {
            return Ok(Some(format!("Z{}", MAX_DIGIT as char)));
        }
        if head == b'A' {
            return Ok(None);
        }
        let h = head - 1;
        if h < b'Z' {
            digs.push(MAX_DIGIT);
        } else {
            digs.pop();
        }
        Ok(Some(make_int(h, &digs)))
    } else {
        Ok(Some(make_int(head, &digs)))
    }
}

fn make_int(head: u8, digits: &[u8]) -> String {
    let mut s = String::with_capacity(1 + digits.len());
    s.push(head as char);
    for &d in digits {
        s.push(d as char);
    }
    s
}

/// Key that sorts strictly between `a` and `b`. `a == None` prepends, `b == None`
/// appends, both `None` yields the canonical first key `"a0"`. `Err` if `a`/`b`
/// is malformed or `a >= b`.
pub fn generate_key_between(a: Option<&str>, b: Option<&str>) -> Result<String, String> {
    if let Some(a) = a {
        validate_order_key(a)?;
    }
    if let Some(b) = b {
        validate_order_key(b)?;
    }
    if let (Some(a), Some(b)) = (a, b) {
        if a >= b {
            return Err(format!("{a:?} >= {b:?}"));
        }
    }

    if a.is_none() {
        let b = match b {
            None => return Ok(format!("a{}", ZERO as char)),
            Some(b) => b,
        };
        let ib = get_integer_part(b)?;
        let fb = &b[ib.len()..];
        if ib == smallest_integer() {
            return Ok(format!("{ib}{}", midpoint("", Some(fb))?));
        }
        if ib < b {
            // `b` has a fraction, so its integer part alone sorts before it.
            return Ok(ib.to_string());
        }
        return match decrement_integer(ib)? {
            Some(res) => Ok(res),
            None => Err("cannot decrement any more".to_string()),
        };
    }
    let a = a.unwrap();

    if b.is_none() {
        let ia = get_integer_part(a)?;
        let fa = &a[ia.len()..];
        return match increment_integer(ia)? {
            None => Ok(format!("{ia}{}", midpoint(fa, None)?)),
            Some(i) => Ok(i),
        };
    }
    let b = b.unwrap();

    let ia = get_integer_part(a)?;
    let fa = &a[ia.len()..];
    let ib = get_integer_part(b)?;
    let fb = &b[ib.len()..];
    if ia == ib {
        return Ok(format!("{ia}{}", midpoint(fa, Some(fb))?));
    }
    let i = match increment_integer(ia)? {
        Some(i) => i,
        None => return Err("cannot increment any more".to_string()),
    };
    if i.as_str() < b {
        return Ok(i);
    }
    Ok(format!("{ia}{}", midpoint(fa, None)?))
}

/// `n` distinct sorted keys, all strictly between `a` and `b`. Both set uses
/// balanced midpoint subdivision so keys stay short. Same error conditions as
/// [`generate_key_between`]; `n == 0` yields an empty vector.
pub fn generate_n_keys_between(
    a: Option<&str>,
    b: Option<&str>,
    n: usize,
) -> Result<Vec<String>, String> {
    if n == 0 {
        return Ok(Vec::new());
    }
    if n == 1 {
        return Ok(vec![generate_key_between(a, b)?]);
    }

    if b.is_none() {
        let mut c = generate_key_between(a, b)?;
        let mut result = vec![c.clone()];
        for _ in 0..n - 1 {
            c = generate_key_between(Some(&c), b)?;
            result.push(c.clone());
        }
        return Ok(result);
    }

    if a.is_none() {
        let mut c = generate_key_between(a, b)?;
        let mut result = vec![c.clone()];
        for _ in 0..n - 1 {
            c = generate_key_between(a, Some(&c))?;
            result.push(c.clone());
        }
        result.reverse();
        return Ok(result);
    }

    let mid = n / 2;
    let c = generate_key_between(a, b)?;
    let mut left = generate_n_keys_between(a, Some(&c), mid)?;
    let right = generate_n_keys_between(Some(&c), b, n - mid - 1)?;
    left.push(c);
    left.extend(right);
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_sorted_unique(keys: &[String]) {
        for w in keys.windows(2) {
            assert_eq!(
                cmp_keys(&w[0], &w[1]),
                Ordering::Less,
                "not strictly sorted: {:?} !< {:?}",
                w[0],
                w[1]
            );
        }
    }

    fn assert_strictly_between(lo: &str, mid: &str, hi: &str) {
        assert_eq!(cmp_keys(lo, mid), Ordering::Less, "expected {lo:?} < {mid:?}");
        assert_eq!(cmp_keys(mid, hi), Ordering::Less, "expected {mid:?} < {hi:?}");
    }

    // Reference golden values from rocicorp/fractional-indexing.

    #[test]
    fn golden_first_and_append() {
        assert_eq!(generate_key_between(None, None).unwrap(), "a0");
        assert_eq!(generate_key_between(Some("a0"), None).unwrap(), "a1");
        assert_eq!(generate_key_between(Some("a1"), None).unwrap(), "a2");
    }

    #[test]
    fn golden_prepend() {
        assert_eq!(generate_key_between(None, Some("a0")).unwrap(), "Zz");
        assert_eq!(generate_key_between(None, Some("Zz")).unwrap(), "Zy");
    }

    #[test]
    fn golden_between_adjacent() {
        assert_eq!(generate_key_between(Some("a0"), Some("a1")).unwrap(), "a0V");
    }

    #[test]
    fn golden_between_prefix() {
        let k = generate_key_between(Some("a0"), Some("a0V")).unwrap();
        assert_strictly_between("a0", &k, "a0V");
    }

    #[test]
    fn golden_n_keys_both_null() {
        assert_eq!(
            generate_n_keys_between(None, None, 5).unwrap(),
            vec!["a0", "a1", "a2", "a3", "a4"]
        );
    }

    #[test]
    fn cmp_keys_is_plain_str_cmp() {
        assert_eq!(cmp_keys("a0", "a1"), Ordering::Less);
        assert_eq!(cmp_keys("a1", "a1"), Ordering::Equal);
        assert_eq!(cmp_keys("a2", "a1"), Ordering::Greater);
        // Negative heads sort before positive (byte 'Z' < 'a').
        assert_eq!(cmp_keys("Zz", "a0"), Ordering::Less);
    }

    #[test]
    fn between_two_adjacent_keys_yields_strictly_between() {
        let a = generate_key_between(None, None).unwrap();
        let b = generate_key_between(Some(&a), None).unwrap();
        let mid = generate_key_between(Some(&a), Some(&b)).unwrap();
        assert_strictly_between(&a, &mid, &b);
    }

    #[test]
    fn append_grows_strictly() {
        let mut keys = vec![generate_key_between(None, None).unwrap()];
        for _ in 0..60 {
            let last = keys.last().cloned();
            let next = generate_key_between(last.as_deref(), None).unwrap();
            assert_eq!(
                cmp_keys(keys.last().unwrap(), &next),
                Ordering::Less,
                "append must strictly increase: {:?} !< {next:?}",
                keys.last().unwrap()
            );
            keys.push(next);
        }
        assert_sorted_unique(&keys);
    }

    #[test]
    fn prepend_shrinks_strictly() {
        let mut keys = vec![generate_key_between(None, None).unwrap()];
        for _ in 0..60 {
            let first = keys.first().cloned();
            let prev = generate_key_between(None, first.as_deref()).unwrap();
            assert_eq!(
                cmp_keys(&prev, keys.first().unwrap()),
                Ordering::Less,
                "prepend must strictly decrease: {prev:?} !< {:?}",
                keys.first().unwrap()
            );
            keys.insert(0, prev);
        }
        assert_sorted_unique(&keys);
    }

    #[test]
    fn repeated_subdivision_stays_ordered() {
        let lo = generate_key_between(None, None).unwrap();
        let hi = generate_key_between(Some(&lo), None).unwrap();
        let mut left = lo.clone();
        let mut right = hi.clone();
        let mut all = vec![lo, hi];
        for step in 0..120 {
            let mid = generate_key_between(Some(&left), Some(&right)).unwrap();
            assert_strictly_between(&left, &mid, &right);
            all.push(mid.clone());
            if step % 2 == 0 {
                right = mid;
            } else {
                left = mid;
            }
        }
        all.sort();
        let before = all.len();
        all.dedup();
        assert_eq!(before, all.len(), "subdivision produced duplicates");
    }

    #[test]
    fn rejects_a_ge_b() {
        assert!(generate_key_between(Some("a1"), Some("a1")).is_err());
        assert!(generate_key_between(Some("a2"), Some("a1")).is_err());
    }

    #[test]
    fn rejects_malformed_key() {
        assert!(generate_key_between(Some(""), None).is_err());
        assert!(validate_order_key("a0V0").is_err());
        assert!(validate_order_key("a0V").is_ok());
        assert!(validate_order_key("a0!").is_err());
        assert!(validate_order_key(&smallest_integer()).is_err());
    }

    #[test]
    fn generate_n_keys_is_monotonic() {
        for n in [0usize, 1, 2, 3, 5, 8, 13, 21, 64] {
            let keys = generate_n_keys_between(None, None, n).unwrap();
            assert_eq!(keys.len(), n, "n={n} produced wrong count");
            assert_sorted_unique(&keys);
        }
    }

    #[test]
    fn generate_n_keys_within_bounds() {
        let a = generate_key_between(None, None).unwrap();
        let b = generate_key_between(Some(&a), None).unwrap();
        let keys = generate_n_keys_between(Some(&a), Some(&b), 25).unwrap();
        assert_eq!(keys.len(), 25);
        assert_eq!(cmp_keys(&a, &keys[0]), Ordering::Less);
        assert_eq!(cmp_keys(keys.last().unwrap(), &b), Ordering::Less);
        assert_sorted_unique(&keys);
    }

    #[test]
    fn generate_n_keys_one_null_bound() {
        let keys = generate_n_keys_between(None, Some("a0"), 4).unwrap();
        assert_eq!(keys.len(), 4);
        assert_sorted_unique(&keys);
        assert_eq!(cmp_keys(keys.last().unwrap(), "a0"), Ordering::Less);

        let keys2 = generate_n_keys_between(Some("a0"), None, 4).unwrap();
        assert_eq!(keys2, vec!["a1", "a2", "a3", "a4"]);
    }

    #[test]
    fn generate_n_keys_zero_and_bad_bounds() {
        assert!(generate_n_keys_between(None, None, 0).unwrap().is_empty());
        assert!(generate_n_keys_between(Some("a5"), Some("a5"), 3).is_err());
    }

    #[test]
    fn deterministic_index_driven_insertions_stay_sorted() {
        let mut keys: Vec<String> = vec![generate_key_between(None, None).unwrap()];
        for i in 0..400usize {
            let len = keys.len();
            let slot = (i * 7 + 3) % (len + 1);
            let left = if slot == 0 { None } else { Some(keys[slot - 1].as_str()) };
            let right = if slot == len { None } else { Some(keys[slot].as_str()) };
            let k = generate_key_between(left, right).unwrap();
            if let Some(l) = left {
                assert_eq!(cmp_keys(l, &k), Ordering::Less, "step {i}: {l:?} !< {k:?}");
            }
            if let Some(r) = right {
                assert_eq!(cmp_keys(&k, r), Ordering::Less, "step {i}: {k:?} !< {r:?}");
            }
            keys.insert(slot, k);
        }
        assert_sorted_unique(&keys);
    }

    #[test]
    fn generated_keys_round_trip_through_validate() {
        let keys = generate_n_keys_between(None, None, 80).unwrap();
        for k in &keys {
            validate_order_key(k)
                .unwrap_or_else(|e| panic!("generated key {k:?} failed validation: {e}"));
        }
    }

    #[test]
    fn many_appends_widen_integer_and_stay_sorted() {
        let mut last = generate_key_between(None, None).unwrap();
        let mut all = vec![last.clone()];
        for _ in 0..700 {
            let next = generate_key_between(Some(&last), None).unwrap();
            assert_eq!(cmp_keys(&last, &next), Ordering::Less, "{last:?} !< {next:?}");
            last = next.clone();
            all.push(next);
        }
        assert_sorted_unique(&all);
        assert!(all.iter().any(|k| k.starts_with('b')), "integer never widened");
    }

    #[test]
    fn increment_then_decrement_round_trips() {
        for x in ["a0", "a1", "az", "Zz", "Zy", "Z0"] {
            if let Some(inc) = increment_integer(x).unwrap() {
                let back = decrement_integer(&inc).unwrap().unwrap();
                assert_eq!(back, x, "inc/dec round trip failed for {x:?}");
            }
        }
    }

    #[test]
    fn negative_side_orders_below_positive() {
        let mut keys = vec![generate_key_between(None, None).unwrap()];
        for _ in 0..70 {
            let first = keys[0].clone();
            let prev = generate_key_between(None, Some(&first)).unwrap();
            keys.insert(0, prev);
        }
        assert_sorted_unique(&keys);
        assert!(keys[0].as_bytes()[0] < b'a', "expected negative head, got {:?}", keys[0]);
    }
}
