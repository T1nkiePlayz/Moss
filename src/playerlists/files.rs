//! The on-disk formats: whitelist.json, banned-players.json (Minecraft Java) and allowlist.json (Bedrock), merged so
//! entries an operator added by hand survive, plus UUID derivation.
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub struct Item { pub name: String, pub uuid: Option<String>, pub reason: String }

/// ASCII case folding, the same as SQLite's `lower()` used by the entry index.
pub fn key(name: &str) -> String { name.to_ascii_lowercase() }

fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 { msg.push(0); }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 { 0 => ((b & c) | (!b & d), i), 1 => ((d & b) | (!d & c), (5 * i + 1) % 16), 2 => (b ^ c ^ d, (3 * i + 5) % 16), _ => (c ^ (b | !d), (7 * i) % 16) };
            let f2 = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d; d = c; c = b;
            b = b.wrapping_add(f2.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a); b0 = b0.wrapping_add(b); c0 = c0.wrapping_add(c); d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() { out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes()); }
    out
}

/// The UUID an offline-mode Minecraft server gives a player: version 3 (MD5) of "OfflinePlayer:<name>".
pub fn offline_uuid(name: &str) -> String {
    let mut h = md5(format!("OfflinePlayer:{name}").as_bytes());
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    dashed(&h.iter().map(|b| format!("{b:02x}")).collect::<String>()).unwrap_or_default()
}

/// 32 hex digits to the 8-4-4-4-12 form.
pub fn dashed(hex: &str) -> Option<String> {
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) { return None; }
    let h = hex.to_lowercase();
    Some(format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]))
}

/// Online-mode UUID from Mojang. None when the name is unknown, malformed, or Mojang does not answer in time.
pub async fn mojang_uuid(http: &reqwest::Client, base: &str, name: &str, timeout: std::time::Duration) -> Option<String> {
    if name.is_empty() || name.len() > 16 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return None; }
    let r = http.get(format!("{base}/users/profiles/minecraft/{name}")).timeout(timeout).send().await.ok()?;
    if !r.status().is_success() { return None; }
    dashed(r.json::<Value>().await.ok()?["id"].as_str()?)
}

/// `existing` JSON array with the entries Moss manages replaced: entries named in `applied` (put there by Moss earlier)
/// or in `desired` are dropped, then every desired item is appended. Everything else is kept untouched, in order.
/// Err when the file holds something other than a JSON array (it is left alone).
pub fn merge(existing: &str, desired: &[Item], applied: &HashSet<String>, entry: impl Fn(&Item) -> Value) -> Result<String, String> {
    let mut list: Vec<Value> = if existing.trim().is_empty() { vec![] } else { serde_json::from_str(existing).map_err(|e| format!("not a JSON list: {e}"))? };
    let want: HashSet<String> = desired.iter().map(|i| key(&i.name)).collect();
    list.retain(|v| v["name"].as_str().is_none_or(|n| { let k = key(n); !applied.contains(&k) && !want.contains(&k) }));
    list.extend(desired.iter().map(entry));
    Ok(serde_json::to_string_pretty(&list).unwrap_or_default() + "\n")
}

/// Names in a JSON list file (lowercased key to the entry's uuid, if any). Empty when the file is missing or unreadable.
pub fn names_in(existing: &str) -> std::collections::HashMap<String, Option<String>> {
    serde_json::from_str::<Vec<Value>>(existing).unwrap_or_default().iter()
        .filter_map(|v| Some((key(v["name"].as_str()?), v["uuid"].as_str().map(String::from)))).collect()
}

pub fn whitelist_entry(i: &Item) -> Value { json!({"uuid": i.uuid, "name": i.name}) }
pub fn ban_entry(i: &Item) -> Value {
    let reason = if i.reason.is_empty() { "Banned by an operator." } else { i.reason.as_str() };
    json!({"uuid": i.uuid, "name": i.name, "created": "1970-01-01 00:00:00 +0000", "source": "Moss", "expires": "forever", "reason": reason})
}
pub fn bedrock_entry(i: &Item) -> Value { json!({"name": i.name, "ignoresPlayerLimit": false}) }

pub struct Diff { pub add: Vec<Item>, pub remove: Vec<String> }

/// What to do to take a server from `applied` (names Moss put there) to `desired`.
pub fn diff(desired: &[Item], applied: &[String]) -> Diff {
    let have: HashSet<String> = applied.iter().map(|n| key(n)).collect();
    let want: HashSet<String> = desired.iter().map(|i| key(&i.name)).collect();
    Diff {
        add: desired.iter().filter(|i| !have.contains(&key(&i.name))).cloned().collect(),
        remove: applied.iter().filter(|n| !want.contains(&key(n))).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn it(n: &str, u: Option<&str>) -> Item { Item { name: n.into(), uuid: u.map(String::from), reason: String::new() } }

    #[test]
    fn offline_uuid_matches_the_known_vector() {
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(md5(b"").iter().map(|b| format!("{b:02x}")).collect::<String>(), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn merge_keeps_hand_added_entries_and_replaces_moss_ones() {
        let existing = r#"[{"uuid":"u-hand","name":"Hand"},{"uuid":"u-old","name":"OldMoss"},{"uuid":"u-x","name":"alice"}]"#;
        let applied: HashSet<String> = ["oldmoss".to_string()].into();
        let out = merge(existing, &[it("Alice", Some("u-alice")), it("Bob", Some("u-bob"))], &applied, whitelist_entry).unwrap();
        let v: Vec<Value> = serde_json::from_str(&out).unwrap();
        let names: Vec<&str> = v.iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Hand", "Alice", "Bob"]);
        assert_eq!(v[1]["uuid"], "u-alice");
        // idempotent
        let applied2: HashSet<String> = ["alice".to_string(), "bob".to_string()].into();
        assert_eq!(merge(&out, &[it("Alice", Some("u-alice")), it("Bob", Some("u-bob"))], &applied2, whitelist_entry).unwrap(), out);
        // removing everything Moss added leaves only the hand entry
        let none = merge(&out, &[], &applied2, whitelist_entry).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Value>>(&none).unwrap().len(), 1);
    }

    #[test]
    fn merge_formats_bans_and_bedrock_and_refuses_garbage() {
        let b = Item { name: "Grief".into(), uuid: Some("u".into()), reason: "griefing".into() };
        let v: Vec<Value> = serde_json::from_str(&merge("", &[b], &HashSet::new(), ban_entry).unwrap()).unwrap();
        assert_eq!((v[0]["reason"].as_str(), v[0]["expires"].as_str(), v[0]["source"].as_str()), (Some("griefing"), Some("forever"), Some("Moss")));
        let v: Vec<Value> = serde_json::from_str(&merge("[]", &[it("Steve", None)], &HashSet::new(), bedrock_entry).unwrap()).unwrap();
        assert_eq!(v[0], json!({"name": "Steve", "ignoresPlayerLimit": false}));
        assert!(merge("{\"a\":1}", &[], &HashSet::new(), bedrock_entry).is_err());
    }

    #[test]
    fn diff_is_case_insensitive() {
        let d = diff(&[it("Alice", None), it("Bob", None)], &["alice".into(), "Carol".into()]);
        assert_eq!(d.add.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Bob"]);
        assert_eq!(d.remove, ["Carol"]);
        let d = diff(&[it("A", None)], &["a".into()]);
        assert!(d.add.is_empty() && d.remove.is_empty());
    }
}
