//! Palworld's `PalWorldSettings.ini`: all settings live in one `OptionSettings=(Key=Value,Key="a, b",...)` tuple. The
//! tuple is split at top-level commas only (quotes and nested parentheses are respected) and an edit replaces the
//! text of one entry, so every untouched entry stays byte for byte as it was.
use serde_json::{Map, Value};

struct Entry { key: String, start: usize, end: usize, val: usize }
struct Tuple { entries: Vec<Entry>, close: usize }

fn locate(raw: &str) -> Result<Option<Tuple>, String> {
    let mut found = None;
    let mut off = 0;
    for l in raw.split_inclusive('\n') {
        let b = l.trim_end_matches(['\n', '\r']);
        if let Some(rest) = b.trim_start().strip_prefix("OptionSettings") && let Some(after_eq) = rest.trim_start().strip_prefix('=') {
            let a = after_eq.trim_start();
            if a.starts_with('(') {
                let open = off + (b.len() - a.len()) + 1;
                let close = b.trim_end().rfind(')').map(|c| off + c).filter(|c| *c >= open).ok_or("OptionSettings is not closed on its line; edit it in the raw editor")?;
                found = Some((open, close));
            }
        }
        off += l.len();
    }
    let Some((open, close)) = found else { return Ok(None) };
    let (mut entries, mut start, mut depth, mut quoted) = (vec![], open, 0usize, false);
    let mut push = |s: usize, e: usize| {
        let piece = &raw[s..e];
        if let Some(eq) = piece.find('=') { entries.push(Entry { key: piece[..eq].trim().to_string(), start: s, end: e, val: s + eq + 1 }); }
    };
    for (i, c) in raw[open..close].char_indices() {
        let i = open + i;
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth = depth.saturating_sub(1),
            ',' if !quoted && depth == 0 => { push(start, i); start = i + 1; }
            _ => {}
        }
    }
    if !raw[start..close].trim().is_empty() { push(start, close); }
    Ok(Some(Tuple { entries, close }))
}

fn plain(v: &str) -> String { let v = v.trim(); v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v).to_string() }

pub fn parse(raw: &str) -> Result<Map<String, Value>, String> {
    let mut m = Map::new();
    if let Some(t) = locate(raw)? { for e in t.entries { m.insert(e.key, Value::String(plain(&raw[e.val..e.end]))); } }
    Ok(m)
}

/// Set `key` to `text`. Strings are quoted when the entry was quoted, when `default_quote` says so for a new entry, or
/// when the text would otherwise break the tuple.
pub fn set(raw: &str, key: &str, text: &str, default_quote: bool) -> Result<String, String> {
    if text.contains(['"', '\n', '\r']) { return Err(format!("{key} cannot contain quotes or line breaks")); }
    let t = locate(raw)?.ok_or("this file has no OptionSettings line; edit it in the raw editor")?;
    let old = t.entries.iter().rfind(|e| e.key == key);
    let was_quoted = old.map(|e| raw[e.val..e.end].trim_start().starts_with('"'));
    let lit = if was_quoted.unwrap_or(default_quote) || text.contains([',', '(', ')', ' ']) { format!("\"{text}\"") } else { text.to_string() };
    Ok(match old {
        Some(e) => format!("{}{lit}{}", &raw[..e.val], &raw[e.end..]),
        None => {
            let sep = if t.entries.is_empty() { "" } else { "," };
            format!("{}{sep}{key}={lit}{}", &raw[..t.close], &raw[t.close..])
        }
    })
}

pub fn unset(raw: &str, key: &str) -> Result<String, String> {
    let Some(t) = locate(raw)? else { return Ok(raw.to_string()) };
    let Some(i) = t.entries.iter().rposition(|e| e.key == key) else { return Ok(raw.to_string()) };
    let e = &t.entries[i];
    let (from, to) = if i > 0 { (t.entries[i - 1].end, e.end) } else if let Some(n) = t.entries.get(1) { (e.start, n.start) } else { (e.start, e.end) };
    Ok(format!("{}{}", &raw[..from], &raw[to..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    const DOC: &str = "[/Script/Pal.PalGameWorldSettings]\nOptionSettings=(Difficulty=None,ExpRate=1.000000,ServerName=\"Moss, the (best) server\",bIsPvP=False,CrossplayPlatforms=(Steam,Xbox),ServerPassword=\"\")\n";
    #[test]
    fn splits_at_top_level_commas_only() {
        let m = parse(DOC).unwrap();
        assert_eq!((m["ServerName"].as_str(), m["Difficulty"].as_str(), m["bIsPvP"].as_str(), m["CrossplayPlatforms"].as_str(), m["ServerPassword"].as_str()), (Some("Moss, the (best) server"), Some("None"), Some("False"), Some("(Steam,Xbox)"), Some("")));
        assert_eq!(m.len(), 6);
    }
    #[test]
    fn edits_replace_one_entry_and_keep_the_rest() {
        let out = set(DOC, "ExpRate", "2.500000", false).unwrap();
        assert_eq!(out, DOC.replace("ExpRate=1.000000", "ExpRate=2.500000"));
        let out = set(DOC, "ServerName", "A, B", true).unwrap();
        assert_eq!(out, DOC.replace("\"Moss, the (best) server\"", "\"A, B\""));
        assert_eq!(set(DOC, "bIsPvP", "True", false).unwrap(), DOC.replace("bIsPvP=False", "bIsPvP=True"));
        assert_eq!(set(DOC, "ServerPassword", "pw", false).unwrap(), DOC.replace("ServerPassword=\"\"", "ServerPassword=\"pw\""), "an entry that was quoted stays quoted");
        assert_eq!(set(DOC, "Difficulty", "Hard", true).unwrap(), DOC.replace("Difficulty=None", "Difficulty=Hard"), "an unquoted entry stays unquoted");
    }
    #[test]
    fn adds_removes_and_rejects() {
        let out = set(DOC, "DayTimeSpeedRate", "1.500000", false).unwrap();
        assert!(out.contains("ServerPassword=\"\",DayTimeSpeedRate=1.500000)\n"));
        assert_eq!(parse(&out).unwrap().len(), 7);
        assert_eq!(unset(DOC, "Difficulty").unwrap(), DOC.replace("Difficulty=None,", ""));
        assert_eq!(unset(DOC, "ServerPassword").unwrap(), DOC.replace(",ServerPassword=\"\"", ""));
        assert_eq!(unset(DOC, "ServerName").unwrap(), DOC.replace(",ServerName=\"Moss, the (best) server\"", ""));
        assert_eq!(unset(DOC, "Nope").unwrap(), DOC);
        assert!(set(DOC, "ServerName", "a\"b", true).is_err());
        assert_eq!(set("OptionSettings=()\n", "A", "1", false).unwrap(), "OptionSettings=(A=1)\n");
        assert!(set("[x]\n", "A", "1", false).is_err());
        assert!(parse("[x]\n").unwrap().is_empty());
        assert!(parse("OptionSettings=(A=1\n").is_err());
    }
    #[test]
    fn crlf_and_spacing_survive() {
        let d = "[s]\r\nOptionSettings=(A=1,B=\"x\")\r\n";
        assert_eq!(set(d, "A", "2", false).unwrap(), d.replace("A=1", "A=2"));
        assert_eq!(parse(d).unwrap()["B"], "x");
    }
}
