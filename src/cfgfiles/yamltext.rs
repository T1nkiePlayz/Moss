//! Comment-preserving edits of block-style YAML text. The file is never re-serialised: a single line is replaced,
//! inserted or removed and everything else (comments, spacing, key order, unknown keys) stays byte for byte.
//! Supported: scalar values in nested block mappings, which is what Paper, Spigot, Bukkit and Purpur write.
//! Anything else (values inside lists, flow mappings, multi-line strings) is refused with a message that points
//! to the raw editor instead of guessing.
use serde_json::Value;

struct Info { idx: usize, indent: usize, path: Vec<String>, colon: usize, inline: bool }

fn split(raw: &str) -> Vec<(&str, &str)> {
    raw.split_inclusive('\n').map(|l| { let b = l.trim_end_matches(['\n', '\r']); (b, &l[b.len()..]) }).collect()
}
fn indent_of(b: &str) -> usize { b.len() - b.trim_start_matches(' ').len() }
fn blank_or_comment(b: &str) -> bool { let t = b.trim(); t.is_empty() || t.starts_with('#') }
fn is_item(t: &str) -> bool { t == "-" || t.starts_with("- ") }

/// Parse `key:` at the start of a trimmed line: (key, byte index of the colon within `t`).
fn parse_key(t: &str) -> Option<(String, usize)> {
    if t.is_empty() || t.starts_with('#') || is_item(t) || t.starts_with("---") || t.starts_with(['{', '[', '?', '|', '>', '&', '*', '!', '%', '@', '`']) { return None; }
    if t.starts_with(['"', '\'']) {
        let q = t.as_bytes()[0] as char;
        let end = t[1..].find(q)? + 1;
        let rest = &t[end + 1..];
        let colon = end + 1 + (rest.len() - rest.trim_start().len());
        return t[colon..].starts_with(':').then(|| (t[1..end].to_string(), colon));
    }
    for (i, c) in t.char_indices() {
        if c == '#' && i > 0 && t.as_bytes()[i - 1] == b' ' { return None; }
        if c == ':' && (t[i + 1..].is_empty() || t[i + 1..].starts_with(' ')) { return Some((t[..i].trim_end().to_string(), i)); }
    }
    None
}

/// Start of a trailing comment in a value (a `#` after whitespace, outside quotes).
fn comment_at(v: &str) -> Option<usize> {
    let (mut q, mut prev_ws, mut esc) = (None::<char>, true, false);
    for (i, c) in v.char_indices() {
        match q {
            Some(d) => { if esc { esc = false } else if c == '\\' && d == '"' { esc = true } else if c == d { q = None } }
            None => { if (c == '"' || c == '\'') && prev_ws { q = Some(c) } else if c == '#' && prev_ws { return Some(i) } }
        }
        prev_ws = c.is_whitespace();
    }
    None
}

fn scan(lines: &[(&str, &str)]) -> Vec<Info> {
    let (mut out, mut stack, mut skip): (Vec<Info>, Vec<(usize, String)>, Option<usize>) = (vec![], vec![], None);
    for (idx, (b, _)) in lines.iter().enumerate() {
        let ind = indent_of(b);
        if let Some(s) = skip { if b.trim().is_empty() || ind > s { continue; } skip = None; }
        let Some((key, colon)) = parse_key(&b[ind..]) else { continue };
        while stack.last().is_some_and(|(i, _)| *i >= ind) { stack.pop(); }
        stack.push((ind, key));
        let rest = b[ind + colon + 1..].trim_start();
        let inline = !rest.is_empty() && !rest.starts_with('#');
        if inline && rest.starts_with(['|', '>']) { skip = Some(ind); }
        out.push(Info { idx, indent: ind, path: stack.iter().map(|(_, k)| k.clone()).collect(), colon: ind + colon, inline });
    }
    out
}

fn eol_of(raw: &str) -> &'static str { if raw.contains("\r\n") { "\r\n" } else { "\n" } }
fn join(lines: Vec<String>) -> String { lines.concat() }

/// Set the scalar at `path` to the YAML literal `lit`, keeping the line's trailing comment.
pub fn set(raw: &str, path: &[&str], lit: &str) -> Result<String, String> {
    let lines = split(raw);
    let infos = scan(&lines);
    let want: Vec<String> = path.iter().map(|s| s.to_string()).collect();
    let name = path.join(".");
    if let Some(i) = infos.iter().find(|i| i.path == want) {
        if !i.inline { return Err(format!("{name} is a section or a list, not a single value; edit it in the raw editor")); }
        let (b, eol) = lines[i.idx];
        let head = &b[..i.colon + 1];
        let v = b[i.colon + 1..].trim_start();
        if v.starts_with(['[', '{']) && !(v.ends_with(']') || v.ends_with('}')) { return Err(format!("{name} spans several lines; edit it in the raw editor")); }
        let tail = comment_at(v).map(|c| format!("{}{}", &v[v[..c].trim_end().len()..c], &v[c..])).unwrap_or_default();
        let mut out: Vec<String> = lines.iter().map(|(b, e)| format!("{b}{e}")).collect();
        out[i.idx] = format!("{head} {lit}{tail}{eol}");
        return Ok(join(out));
    }
    let nl = eol_of(raw);
    let mut out: Vec<String> = lines.iter().map(|(b, e)| format!("{b}{e}")).collect();
    let best = (1..path.len()).rev().find_map(|k| infos.iter().find(|i| i.path == want[..k]).map(|i| (k, i)));
    let (k, at, pad) = match best {
        None => (0, out.len(), 0),
        Some((k, i)) => {
            if i.inline { return Err(format!("{} holds a value rather than a section; edit it in the raw editor", path[..k].join("."))); }
            let (mut last, mut child) = (i.idx, None);
            for (j, (b, _)) in lines.iter().enumerate().skip(i.idx + 1) {
                if blank_or_comment(b) { continue; }
                let ind = indent_of(b);
                if ind <= i.indent && !(ind == i.indent && is_item(b.trim_start())) { break; }
                last = j;
                if child.is_none() && ind > i.indent { child = Some(ind); }
            }
            (k, last + 1, child.unwrap_or(i.indent + 2))
        }
    };
    if at > 0 && !out[at - 1].ends_with('\n') { out[at - 1].push_str(nl); }
    let mut add = String::new();
    let mut p = pad;
    for seg in &path[k..path.len() - 1] { add += &format!("{}{seg}:{nl}", " ".repeat(p)); p += 2; }
    add += &format!("{}{}: {lit}{nl}", " ".repeat(p), path[path.len() - 1]);
    out.insert(at, add);
    Ok(join(out))
}

/// Remove the scalar at `path` so the server falls back to its default. Missing keys are left alone.
pub fn unset(raw: &str, path: &[&str]) -> Result<String, String> {
    let lines = split(raw);
    let infos = scan(&lines);
    let want: Vec<String> = path.iter().map(|s| s.to_string()).collect();
    let Some(i) = infos.iter().find(|i| i.path == want) else { return Ok(raw.to_string()) };
    if !i.inline { return Err(format!("{} is a section or a list; edit it in the raw editor", path.join("."))); }
    let mut end = i.idx + 1;
    let (b, _) = lines[i.idx];
    if b[i.colon + 1..].trim_start().starts_with(['|', '>']) { while end < lines.len() && (lines[end].0.trim().is_empty() || indent_of(lines[end].0) > i.indent) { end += 1; } }
    Ok(lines.iter().enumerate().filter(|(j, _)| *j < i.idx || *j >= end).map(|(_, (b, e))| format!("{b}{e}")).collect())
}

/// YAML text for a JSON scalar. `float` keeps whole numbers looking like floats (`10.0`).
pub fn literal(v: &Value, float: bool) -> Result<String, String> {
    Ok(match v {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) if float => format!("{:?}", n.as_f64().ok_or("number out of range")?),
        Value::Number(n) => n.to_string(),
        Value::String(s) => plain_or_quoted(s),
        _ => return Err("only text, numbers and true/false can be set here".into()),
    })
}

fn plain_or_quoted(s: &str) -> String {
    let safe = !s.is_empty() && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'))
        && s.parse::<f64>().is_err() && !matches!(s.to_ascii_lowercase().as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n");
    if safe { s.to_string() } else { serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const DOC: &str = "# Paper config\nconfig-version: 28\n\nchunks:\n  # how many\n  max-auto-save-chunks-per-tick: 24 # default\n  delay-chunk-unloads-by: 10s\n  nested:\n    deep: true\n\nmisc:\n  list:\n    - a\n    - b\n  note: |\n    chunks:\n      fake: 1\n  last: 1\n";

    fn parsed(s: &str) -> Value { serde_yaml_ng::from_str(s).unwrap() }

    #[test]
    fn replaces_value_and_keeps_comment() {
        let o = set(DOC, &["chunks", "max-auto-save-chunks-per-tick"], "48").unwrap();
        assert!(o.contains("  max-auto-save-chunks-per-tick: 48 # default\n"), "{o}");
        assert_eq!(o.replace("48 # default", "24 # default"), DOC, "nothing else changes");
        assert_eq!(parsed(&o)["chunks"]["max-auto-save-chunks-per-tick"], 48);
    }
    #[test]
    fn inserts_into_existing_section_with_its_indent() {
        let o = set(DOC, &["chunks", "prevent"], "true").unwrap();
        assert_eq!(parsed(&o)["chunks"]["prevent"], true);
        assert!(o.contains("    deep: true\n  prevent: true\n") || o.contains("  prevent: true\n"), "{o}");
        let p = parsed(&o);
        assert_eq!(p["chunks"]["nested"]["deep"], true, "child of nested stays under nested");
    }
    #[test]
    fn inserts_missing_sections_and_appends_after_lists() {
        let o = set(DOC, &["new", "inner", "key"], "\"x y\"").unwrap();
        assert_eq!(parsed(&o)["new"]["inner"]["key"], "x y");
        let o = set(DOC, &["misc", "extra"], "5").unwrap();
        let p = parsed(&o);
        assert_eq!(p["misc"]["extra"], 5);
        assert_eq!(p["misc"]["list"], json!(["a", "b"]));
        assert_eq!(p["misc"]["last"], 1);
    }
    #[test]
    fn block_scalar_content_is_not_mistaken_for_keys() {
        let o = set(DOC, &["misc", "note", "fake"], "1");
        assert!(o.is_err(), "note holds a value, so a child cannot be added");
        let o = set(DOC, &["chunks", "fake"], "2").unwrap();
        assert_eq!(parsed(&o)["chunks"]["fake"], 2);
    }
    #[test]
    fn refuses_sections_and_missing_final_newline_is_handled() {
        assert!(set(DOC, &["chunks"], "1").unwrap_err().contains("section"));
        let o = set("a: 1", &["b"], "2").unwrap();
        assert_eq!(o, "a: 1\nb: 2\n");
        assert_eq!(set("", &["a", "b"], "1").unwrap(), "a:\n  b: 1\n");
    }
    #[test]
    fn crlf_is_preserved() {
        let o = set("a:\r\n  b: 1\r\n", &["a", "c"], "2").unwrap();
        assert_eq!(o, "a:\r\n  b: 1\r\n  c: 2\r\n");
        assert_eq!(set("a:\r\n  b: 1\r\n", &["a", "b"], "3").unwrap(), "a:\r\n  b: 3\r\n");
    }
    #[test]
    fn quoted_keys_and_hash_inside_strings() {
        let doc = "\"minecraft:zombie\": 1\ns: \"a # not a comment\" # real\n";
        assert_eq!(set(doc, &["minecraft:zombie"], "2").unwrap(), "\"minecraft:zombie\": 2\ns: \"a # not a comment\" # real\n");
        assert_eq!(set(doc, &["s"], "x").unwrap(), "\"minecraft:zombie\": 1\ns: x # real\n");
    }
    #[test]
    fn unset_removes_only_the_key() {
        let o = unset(DOC, &["chunks", "max-auto-save-chunks-per-tick"]).unwrap();
        assert!(!o.contains("max-auto-save") && o.contains("delay-chunk-unloads-by"));
        assert_eq!(unset(DOC, &["nope"]).unwrap(), DOC);
        assert!(unset(DOC, &["chunks"]).is_err());
        let o = unset(DOC, &["misc", "note"]).unwrap();
        assert_eq!(parsed(&o)["misc"]["last"], 1);
        assert!(!o.contains("fake"));
    }
    #[test]
    fn literals() {
        assert_eq!(literal(&json!(true), false).unwrap(), "true");
        assert_eq!(literal(&json!(10), true).unwrap(), "10.0");
        assert_eq!(literal(&json!(0.0625), true).unwrap(), "0.0625");
        assert_eq!(literal(&json!("10s"), false).unwrap(), "10s");
        assert_eq!(literal(&json!("123"), false).unwrap(), "\"123\"");
        assert_eq!(literal(&json!("yes"), false).unwrap(), "\"yes\"");
        assert_eq!(literal(&json!("<red>Hi: you #1"), false).unwrap(), "\"<red>Hi: you #1\"");
        assert!(literal(&json!(null), false).is_err() && literal(&json!([1]), false).is_err());
        for s in ["a: b", "- x", "", "'q'", "tab\there", "uni\u{e9}", "back\\slash"] {
            let d = format!("k: {}\n", literal(&json!(s), false).unwrap());
            assert_eq!(parsed(&d)["k"], s, "{d}");
        }
    }
}
