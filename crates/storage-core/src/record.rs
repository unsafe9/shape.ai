//! Store-neutral data model: opaque, versioned byte-payload [`Record`]s the
//! storage core moves without knowing about scenes, cards, or edges.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One opaque, addressable, versioned unit of stored data. The byte-oriented
/// `payload` is what lets one portable format carry every adapter losslessly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    /// Caller-defined category for the record.
    pub kind: String,
    /// Monotonic revision; callers may use it for optimistic concurrency.
    pub version: u64,
    #[serde(with = "bytes_as_base64")]
    pub payload: Vec<u8>,
}

impl Record {
    /// Build a version-1 record from id, kind and payload bytes.
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        payload: impl Into<Vec<u8>>,
    ) -> Self {
        Record {
            id: id.into(),
            kind: kind.into(),
            version: 1,
            payload: payload.into(),
        }
    }
}

/// The full logical contents of a store at one instant: the in-memory shape
/// every adapter can [`snapshot`] and [`restore`]. The `BTreeMap` keying gives
/// deterministic id order, which makes the exported format byte-stable.
///
/// [`snapshot`]: crate::StorageAdapter::snapshot
/// [`restore`]: crate::StorageAdapter::restore
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreSnapshot {
    records: BTreeMap<String, Record>,
}

impl StoreSnapshot {
    pub fn new() -> Self {
        StoreSnapshot::default()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Insert or replace a record, returning the previous value if any.
    pub fn insert(&mut self, record: Record) -> Option<Record> {
        self.records.insert(record.id.clone(), record)
    }

    /// Remove a record by id, returning it if present.
    pub fn remove(&mut self, id: &str) -> Option<Record> {
        self.records.remove(id)
    }

    pub fn get(&self, id: &str) -> Option<&Record> {
        self.records.get(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.records.contains_key(id)
    }

    /// All record ids, in sorted order.
    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.records.keys()
    }

    /// Records in id-sorted order. The format relies on this for byte stability.
    pub fn records(&self) -> impl Iterator<Item = &Record> {
        self.records.values()
    }

    pub fn into_records(self) -> Vec<Record> {
        self.records.into_values().collect()
    }

    /// Build a snapshot from an iterator of records (later ids win on clash).
    pub fn from_records<I: IntoIterator<Item = Record>>(records: I) -> Self {
        let mut snap = StoreSnapshot::new();
        for r in records {
            snap.insert(r);
        }
        snap
    }
}

/// Payloads serialize as base64 in the JSON representation only; the binary
/// shard format frames the raw bytes directly.
mod bytes_as_base64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        decode(&text).map_err(serde::de::Error::custom)
    }

    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    /// Minimal, dependency-free standard base64 encoder (with padding).
    pub fn encode(input: &[u8]) -> String {
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[((n >> 6) & 0x3f) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(n & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    #[allow(clippy::cast_possible_truncation, reason = "base64 decode extracts bytes from a packed u32")]
    pub fn decode(input: &str) -> Result<Vec<u8>, String> {
        fn val(c: u8) -> Result<u32, String> {
            match c {
                b'A'..=b'Z' => Ok((c - b'A') as u32),
                b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
                b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
                b'+' => Ok(62),
                b'/' => Ok(63),
                _ => Err(format!("invalid base64 byte: {c}")),
            }
        }
        let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        if bytes.len() % 4 != 0 {
            return Err("base64 length not a multiple of 4".into());
        }
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for chunk in bytes.chunks(4) {
            let pad = chunk.iter().filter(|&&c| c == b'=').count();
            let n = (val(chunk[0])? << 18)
                | (val(chunk[1])? << 12)
                | (if chunk[2] == b'=' { 0 } else { val(chunk[2])? } << 6)
                | (if chunk[3] == b'=' { 0 } else { val(chunk[3])? });
            out.push((n >> 16) as u8);
            if pad < 2 {
                out.push((n >> 8) as u8);
            }
            if pad < 1 {
                out.push(n as u8);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::cast_possible_truncation, reason = "test fixtures intentionally truncate to byte values")]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrips_all_lengths() {
        for n in 0..200usize {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 31 % 256) as u8).collect();
            let enc = bytes_as_base64::encode(&bytes);
            let dec = bytes_as_base64::decode(&enc).expect("decode");
            assert_eq!(bytes, dec, "roundtrip failed at len {n}");
        }
    }

    #[test]
    fn snapshot_dedup_and_order() {
        let snap = StoreSnapshot::from_records([
            Record::new("b", "card", b"2".to_vec()),
            Record::new("a", "card", b"1".to_vec()),
            Record::new("a", "card", b"1-new".to_vec()),
        ]);
        assert_eq!(snap.len(), 2);
        let ids: Vec<&String> = snap.ids().collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(snap.get("a").unwrap().payload, b"1-new");
    }
}
