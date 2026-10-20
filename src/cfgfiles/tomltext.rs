//! Comment-preserving edits of TOML text (velocity.toml). Same approach as `yamltext`: change one line, leave the
//! rest of the file untouched. A path is `[table, ..., key]`; the top level has no table.
use serde_json::Value;

enum Ln<'a> { Table(Vec<String>), Key(&'a str, usize), Other }

fn unquote(k: &str) -> String { let k = k.trim(); if k.len() >= 2 && (k.starts_with('"') && k.ends_with('"') || k.starts_with('\'') && k.ends_with('\'')) { k[1..k.len() - 1].to_string() } else { k.to_string() } }

fn classify(b: &str) -> Ln<'_> {
    let t = b.trim();
    if t.starts_with("[[") { return Ln::Table(vec!["[[".into(), t.to_string()]); } // arrays of tables are never edited
    if let Some(h) = t.strip_prefix('[') {
        let h = h.split(']').next().unwrap_or_default();
        return Ln::Table(h.split('.').map(unquote).collect());
    }
    if t.is_empty() || t.starts_with('#') { return Ln::Other; }
    match b.find('=') { Some(eq) if !t.starts_with(['"', '\'']) || b[..eq].trim().ends_with(['"', '\'']) => Ln::Key(b[..eq].trim_end(), eq), _ => Ln::Other }
}

fn comment_at(v: &str) -> Option<usize> {
    let (mut q, mut esc) = (None::<char>, false);
    for (i, c) in v.char_indices() {
        match q {
            Some(d) => { if esc { esc = false } else if c == '\\' && d == '"' { esc = true } else if c == d { q = None } }
            None => { if c == '"' || c == '\'' { q = Some(c) } else if c == '#' { return Some(i) } }
        }
    }
    None
}

fn lines(raw: &str) -> Vec<(&str, &str)> { raw.split_inclusive('\n').map(|l| { let b = l.trim_end_matches(['\n', '\r']); (b, &l[b.len()..]) }).collect() }

/// (line index, table of that line) for every key line.
fn keys(ls: &[(&str, &str)]) -> Vec<(usize, Vec<String>, String, usize)> {
    let mut table: Vec<String> = vec![];
    let mut out = vec![];
    for (i, (b, _)) in ls.iter().enumerate() {
        match classify(b) { Ln::Table(t) => table = t, Ln::Key(k, eq) => out.push((i, table.clone(), unquote(k), eq)), Ln::Other => {} }
    }
    out
}

pub fn set(raw: &str, path: &[&str], lit: &str) -> Result<String, String> {
    let (key, table) = path.split_last().ok_or("empty key")?;
    let table: Vec<String> = table.iter().map(|s| s.to_string()).collect();
    let ls = lines(raw);
    let nl = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out: Vec<String> = ls.iter().map(|(b, e)| format!("{b}{e}")).collect();
    let name = path.join(".");
    if let Some((i, _, _, eq)) = keys(&ls).into_iter().find(|(_, t, k, _)| *t == table && k == key) {
        let (b, eol) = ls[i];
        let v = b[eq + 1..].trim_start();
        if v.starts_with('[') && !comment_at(v).map_or(v, |c| v[..c].trim_end()).ends_with(']') || v.starts_with("\"\"\"") || v.starts_with("'''") {
            return Err(format!("{name} spans several lines; edit it in the raw editor"));
        }
        let tail = comment_at(v).map(|c| format!("{}{}", &v[v[..c].trim_end().len()..c], &v[c..])).unwrap_or_default();
        out[i] = format!("{} = {lit}{tail}{eol}", &b[..eq].trim_end());
        return Ok(out.concat());
    }
    // find the end of the table's key lines, or the place a new table would go
    let header = ls.iter().position(|(b, _)| matches!(classify(b), Ln::Table(t) if t == table));
    let at = if table.is_empty() {
        // top level: after the last top-level key, else after the leading comments/blank lines
        let first_table = ls.iter().position(|(b, _)| matches!(classify(b), Ln::Table(_))).unwrap_or(ls.len());
        let last_key = (0..first_table).rev().find(|j| matches!(classify(ls[*j].0), Ln::Key(..)));
        last_key.map_or_else(|| (0..first_table).take_while(|j| !matches!(classify(ls[*j].0), Ln::Key(..))).count().min(first_table), |j| j + 1)
    } else if let Some(h) = header {
        let next = ls.iter().enumerate().skip(h + 1).find(|(_, (b, _))| matches!(classify(b), Ln::Table(_))).map_or(ls.len(), |(j, _)| j);
        (h + 1..next).rev().find(|j| matches!(classify(ls[*j].0), Ln::Key(..))).map_or(h + 1, |j| j + 1)
    } else {
        if let Some(l) = out.last_mut() && !l.ends_with('\n') { l.push_str(nl); }
        if !out.is_empty() { out.push(nl.to_string()); }
        out.push(format!("[{}]{nl}", table.join(".")));
        out.len()
    };
    if at > 0 && at <= out.len() && !out[at - 1].ends_with('\n') { out[at - 1].push_str(nl); }
    out.insert(at, format!("{key} = {lit}{nl}"));
    Ok(out.concat())
}

pub fn unset(raw: &str, path: &[&str]) -> Result<String, String> {
    let (key, table) = path.split_last().ok_or("empty key")?;
    let table: Vec<String> = table.iter().map(|s| s.to_string()).collect();
    let ls = lines(raw);
    let Some((i, ..)) = keys(&ls).into_iter().find(|(_, t, k, _)| *t == table && k == key) else { return Ok(raw.to_string()) };
    let v = ls[i].0.split_once('=').map(|(_, v)| v.trim()).unwrap_or_default();
    if v.starts_with('[') && !v.ends_with(']') { return Err(format!("{} spans several lines; edit it in the raw editor", path.join("."))); }
    Ok(ls.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, (b, e))| format!("{b}{e}")).collect())
}

pub fn literal(v: &Value, float: bool) -> Result<String, String> {
    Ok(match v {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) if float => format!("{:?}", n.as_f64().ok_or("number out of range")?),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
        _ => return Err("only text, numbers and true/false can be set here".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const DOC: &str = "# Config version\nconfig-version = \"2.7\"\n\nbind = \"0.0.0.0:25577\" # listen\nmotd = \"hi # there\"\n\n[servers]\nlobby = \"127.0.0.1:30066\"\ntry = [\n  \"lobby\"\n]\n\n[advanced]\n# compression\ncompression-threshold = 256\n\n[query]\nenabled = false\n";
    fn p(s: &str) -> toml::Value { toml::from_str(s).unwrap() }

    #[test]
    fn replaces_top_level_and_table_values_keeping_comments() {
        let o = set(DOC, &["bind"], "\"0.0.0.0:25565\"").unwrap();
        assert!(o.contains("bind = \"0.0.0.0:25565\" # listen\n"), "{o}");
        let o = set(DOC, &["advanced", "compression-threshold"], "-1").unwrap();
        assert_eq!(p(&o)["advanced"]["compression-threshold"].as_integer(), Some(-1));
        assert_eq!(o.replace("= -1", "= 256"), DOC);
        let o = set(DOC, &["motd"], "\"x\"").unwrap();
        assert!(o.contains("motd = \"x\"\n"));
    }
    #[test]
    fn inserts_in_the_right_table() {
        let o = set(DOC, &["advanced", "login-ratelimit"], "3000").unwrap();
        assert_eq!(p(&o)["advanced"]["login-ratelimit"].as_integer(), Some(3000));
        assert!(p(&o).get("login-ratelimit").is_none());
        let o = set(DOC, &["show-max-players"], "500").unwrap();
        assert_eq!(p(&o)["show-max-players"].as_integer(), Some(500));
        let o = set(DOC, &["query", "port"], "25577").unwrap();
        assert_eq!(p(&o)["query"]["port"].as_integer(), Some(25577));
        assert_eq!(p(&o)["query"]["enabled"].as_bool(), Some(false));
    }
    #[test]
    fn creates_missing_tables_and_handles_empty_files() {
        let o = set("a = 1\n", &["query", "enabled"], "true").unwrap();
        assert_eq!(p(&o)["query"]["enabled"].as_bool(), Some(true));
        assert_eq!(set("", &["x"], "1").unwrap(), "x = 1\n");
        assert_eq!(p(&set("# c\n[t]\nk = 1\n", &["top"], "2").unwrap())["top"].as_integer(), Some(2));
        assert_eq!(p(&set("a = 1", &["t", "b"], "2").unwrap())["t"]["b"].as_integer(), Some(2));
    }
    #[test]
    fn multiline_arrays_are_refused() {
        assert!(set(DOC, &["servers", "try"], "[]").is_err());
        assert!(unset(DOC, &["servers", "try"]).is_err());
    }
    #[test]
    fn unset_and_literals() {
        let o = unset(DOC, &["query", "enabled"]).unwrap();
        assert!(!o.contains("enabled"));
        assert_eq!(unset(DOC, &["nope"]).unwrap(), DOC);
        assert_eq!(literal(&json!(10), true).unwrap(), "10.0");
        assert_eq!(literal(&json!("a\"b"), false).unwrap(), "\"a\\\"b\"");
        assert!(literal(&json!(null), false).is_err());
    }
}
