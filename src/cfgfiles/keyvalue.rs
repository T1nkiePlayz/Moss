//! Comment-preserving edits of `key=value` text (moss-game.cfg, Terraria's serverconfig.txt, Project Zomboid's server
//! .ini). Lines starting with `#` are comments. An edit changes one line and leaves every other byte alone.
use serde_json::{Map, Value};

/// Each line split into its text and its line ending, so lines can be put back together byte for byte.
pub(super) fn lines(raw: &str) -> Vec<(&str, &str)> { raw.split_inclusive('\n').map(|l| { let b = l.trim_end_matches(['\n', '\r']); (b, &l[b.len()..]) }).collect() }

/// (key, value, index of the `=`) of a setting line.
fn entry(b: &str) -> Option<(&str, &str, usize)> {
    let t = b.trim_start();
    if t.is_empty() || t.starts_with('#') { return None; }
    let eq = b.find('=')?;
    let key = b[..eq].trim();
    (!key.is_empty()).then(|| (key, b[eq + 1..].trim(), eq))
}

/// Every setting as text (the last one wins when a key repeats).
pub fn parse(raw: &str) -> Map<String, Value> {
    let mut m = Map::new();
    for (b, _) in lines(raw) { if let Some((k, v, _)) = entry(b) { m.insert(k.to_string(), Value::String(v.to_string())); } }
    m
}

pub fn set(raw: &str, key: &str, lit: &str) -> Result<String, String> {
    if lit.contains(['\n', '\r']) { return Err(format!("{key} must be a single line")); }
    let ls = lines(raw);
    let mut out: Vec<String> = ls.iter().map(|(b, e)| format!("{b}{e}")).collect();
    if let Some(i) = ls.iter().rposition(|(b, _)| entry(b).is_some_and(|(k, _, _)| k == key)) {
        let (b, eol) = ls[i];
        let eq = entry(b).map(|e| e.2).unwrap_or_default();
        out[i] = format!("{}={lit}{eol}", &b[..eq]);
        return Ok(out.concat());
    }
    let nl = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    if !raw.is_empty() && !raw.ends_with('\n') { out.push(nl.into()); }
    out.push(format!("{key}={lit}{nl}"));
    Ok(out.concat())
}

pub fn unset(raw: &str, key: &str) -> Result<String, String> {
    Ok(lines(raw).into_iter().filter(|(b, _)| !entry(b).is_some_and(|(k, _, _)| k == key)).map(|(b, e)| format!("{b}{e}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    const DOC: &str = "# Moss settings\nworld=/srv/w.wld\n#maxplayers=16\nmaxplayers = 8\r\npassword=\nmotd=Hi = there\n";
    #[test]
    fn parses_and_keeps_comments_out() {
        let m = parse(DOC);
        assert_eq!((m["maxplayers"].as_str(), m["password"].as_str(), m["motd"].as_str(), m.contains_key("#maxplayers")), (Some("8"), Some(""), Some("Hi = there"), false));
    }
    #[test]
    fn set_changes_one_line_only() {
        let out = set(DOC, "maxplayers", "12").unwrap();
        assert_eq!(out, DOC.replace("maxplayers = 8", "maxplayers =12"));
        assert_eq!(set(DOC, "motd", "Yo").unwrap(), DOC.replace("Hi = there", "Yo"));
        assert_eq!(set(DOC, "maxplayers", "8").unwrap().len(), DOC.len() - 1, "only the spacing around = is normalised");
        assert!(set(DOC, "motd", "a\nb").is_err());
    }
    #[test]
    fn set_appends_and_unset_removes() {
        assert_eq!(set("a=1", "b", "2").unwrap(), "a=1\nb=2\n");
        assert_eq!(set("", "b", "2").unwrap(), "b=2\n");
        assert_eq!(set("a=1\r\n", "b", "2").unwrap(), "a=1\r\nb=2\r\n");
        assert_eq!(unset(DOC, "password").unwrap(), DOC.replace("password=\n", ""));
        assert_eq!(unset(DOC, "nothing").unwrap(), DOC);
        assert!(unset(DOC, "maxplayers").unwrap().contains("#maxplayers=16"), "comments stay");
    }
}
