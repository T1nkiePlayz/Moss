//! Comment-preserving edits of Source engine config text (`cvar "value"` lines, `//` comments), as in `server.cfg`.
//! Cvar names are case-insensitive. An edit rewrites the value of one line (keeping any trailing comment) or appends
//! a new line; every other byte stays.
use super::keyvalue::lines;
use serde_json::{Map, Value};

struct Cmd<'a> { name: &'a str, val: &'a str, start: usize, end: usize }

fn cmd(b: &str) -> Option<Cmd<'_>> {
    let t = b.trim_start();
    if t.is_empty() || t.starts_with("//") { return None; }
    let lead = b.len() - t.len();
    let name_end = t.find(char::is_whitespace).unwrap_or(t.len());
    let name = &t[..name_end];
    if name.contains(['"', ';']) { return None; }
    let rest = &t[name_end..];
    let vs = lead + name_end + (rest.len() - rest.trim_start().len());
    let r = &b[vs..];
    if let Some(q) = r.strip_prefix('"') {
        return Some(match q.find('"') { Some(e) => Cmd { name, val: &q[..e], start: vs, end: vs + e + 2 }, None => Cmd { name, val: q, start: vs, end: b.len() } });
    }
    let e = [r.find(char::is_whitespace), r.find(';'), r.find("//")].into_iter().flatten().min().unwrap_or(r.len());
    Some(Cmd { name, val: &r[..e], start: vs, end: vs + e })
}

pub fn parse(raw: &str) -> Map<String, Value> {
    let mut m = Map::new();
    for (b, _) in lines(raw) { if let Some(c) = cmd(b) && !c.val.is_empty() { m.insert(c.name.to_ascii_lowercase(), Value::String(c.val.to_string())); } }
    m
}

/// `lit` is the complete replacement for the value, normally the quoted text.
pub fn set(raw: &str, name: &str, lit: &str) -> Result<String, String> {
    let ls = lines(raw);
    let mut out: Vec<String> = ls.iter().map(|(b, e)| format!("{b}{e}")).collect();
    if let Some(i) = ls.iter().rposition(|(b, _)| cmd(b).is_some_and(|c| c.name.eq_ignore_ascii_case(name))) {
        let (b, eol) = ls[i];
        let c = cmd(b).ok_or("unreadable line")?;
        let head = &b[..c.start];
        let gap = if head.ends_with(char::is_whitespace) { "" } else { " " };
        out[i] = format!("{head}{gap}{lit}{}{eol}", &b[c.end..]);
        return Ok(out.concat());
    }
    let nl = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    if !raw.is_empty() && !raw.ends_with('\n') { out.push(nl.into()); }
    out.push(format!("{name} {lit}{nl}"));
    Ok(out.concat())
}

pub fn unset(raw: &str, name: &str) -> Result<String, String> {
    Ok(lines(raw).into_iter().filter(|(b, _)| !cmd(b).is_some_and(|c| c.name.eq_ignore_ascii_case(name))).map(|(b, e)| format!("{b}{e}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    const DOC: &str = "// server\nhostname \"My Server\" // name\nsv_cheats 0\nSV_LAN\t\"1\"\nmp_timelimit 30;echo hi\n";
    #[test]
    fn parses_quoted_bare_and_case_insensitive() {
        let m = parse(DOC);
        assert_eq!((m["hostname"].as_str(), m["sv_cheats"].as_str(), m["sv_lan"].as_str(), m["mp_timelimit"].as_str()), (Some("My Server"), Some("0"), Some("1"), Some("30")));
        assert!(!m.contains_key("//"));
    }
    #[test]
    fn set_keeps_comments_and_the_rest_of_the_line() {
        let out = set(DOC, "hostname", "\"New\"").unwrap();
        assert_eq!(out, DOC.replace("\"My Server\"", "\"New\""));
        assert_eq!(set(DOC, "sv_cheats", "\"1\"").unwrap(), DOC.replace("sv_cheats 0", "sv_cheats \"1\""));
        assert_eq!(set(DOC, "sv_lan", "\"0\"").unwrap(), DOC.replace("\"1\"\n", "\"0\"\n"));
        assert_eq!(set(DOC, "mp_timelimit", "\"5\"").unwrap(), DOC.replace("mp_timelimit 30;", "mp_timelimit \"5\";"));
    }
    #[test]
    fn set_appends_creates_and_unsets() {
        assert_eq!(set("", "hostname", "\"x\"").unwrap(), "hostname \"x\"\n");
        assert_eq!(set("a 1", "b", "\"2\"").unwrap(), "a 1\nb \"2\"\n");
        assert_eq!(set("sv_lan\n", "sv_lan", "\"1\"").unwrap(), "sv_lan \"1\"\n");
        assert_eq!(unset(DOC, "sv_cheats").unwrap(), DOC.replace("sv_cheats 0\n", ""));
        assert!(unset(DOC, "hostname").unwrap().starts_with("// server\nsv_cheats"));
    }
}
