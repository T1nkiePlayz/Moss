//! Edits of JSON settings (Factorio's server-settings.json). The document is parsed with key order kept, one value is
//! changed and it is written back with the file's own indentation, so unknown keys and their order survive. A change
//! that does not alter the value returns the original text untouched.
use serde::Serialize;
use serde_json::{Value, ser::{PrettyFormatter, Serializer}};

pub fn parse(raw: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if v.is_object() { Ok(v) } else { Err("the top level must be an object".into()) }
}

fn indent(raw: &str) -> String {
    raw.lines().skip(1).find_map(|l| { let t = l.trim_start_matches([' ', '\t']); (!t.is_empty() && t.len() < l.len()).then(|| l[..l.len() - t.len()].to_string()) }).unwrap_or_else(|| "  ".into())
}

fn render(raw: &str, doc: &Value) -> Result<String, String> {
    let ind = indent(raw);
    let mut buf = vec![];
    let mut ser = Serializer::with_formatter(&mut buf, PrettyFormatter::with_indent(ind.as_bytes()));
    doc.serialize(&mut ser).map_err(|e| e.to_string())?;
    let mut s = String::from_utf8(buf).map_err(|e| e.to_string())?;
    if raw.ends_with('\n') { s.push('\n'); }
    if raw.contains("\r\n") { s = s.replace('\n', "\r\n"); }
    Ok(s)
}

pub fn set(raw: &str, path: &[&str], v: &Value) -> Result<String, String> {
    let mut doc = parse(raw)?;
    let (last, parents) = path.split_last().ok_or("empty key")?;
    let mut cur = &mut doc;
    for p in parents {
        let o = cur.as_object_mut().ok_or_else(|| format!("{} is not an object", path.join(".")))?;
        cur = o.entry(p.to_string()).or_insert_with(|| Value::Object(Default::default()));
    }
    let o = cur.as_object_mut().ok_or_else(|| format!("{} is not an object", path.join(".")))?;
    if o.get(*last) == Some(v) { return Ok(raw.to_string()); }
    o.insert(last.to_string(), v.clone());
    render(raw, &doc)
}

pub fn unset(raw: &str, path: &[&str]) -> Result<String, String> {
    let mut doc = parse(raw)?;
    let (last, parents) = path.split_last().ok_or("empty key")?;
    let mut cur = &mut doc;
    for p in parents { match cur.get_mut(*p) { Some(n) => cur = n, None => return Ok(raw.to_string()) } }
    match cur.as_object_mut().and_then(|o| o.shift_remove(*last)) { Some(_) => render(raw, &doc), None => Ok(raw.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const DOC: &str = "{\n    \"name\": \"Moss\",\n    \"_comment\": \"keep me\",\n    \"visibility\": {\n        \"public\": false,\n        \"lan\": false\n    },\n    \"max_players\": 0,\n    \"zeta\": [\n        1,\n        2\n    ]\n}\n";
    #[test]
    fn set_keeps_order_unknown_keys_and_indent() {
        let out = set(DOC, &["max_players"], &json!(10)).unwrap();
        assert_eq!(out, DOC.replace("\"max_players\": 0", "\"max_players\": 10"), "indent, order and layout are the file's own");
        let keys = |s: &str| parse(s).unwrap().as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&out), ["name", "_comment", "visibility", "max_players", "zeta"]);
        let out = set(DOC, &["visibility", "public"], &json!(true)).unwrap();
        assert_eq!(parse(&out).unwrap()["visibility"], json!({"public": true, "lan": false}));
    }
    #[test]
    fn unchanged_value_returns_identical_bytes() {
        assert_eq!(set(DOC, &["name"], &json!("Moss")).unwrap(), DOC);
        assert_eq!(unset(DOC, &["nothing"]).unwrap(), DOC);
        assert_eq!(unset(DOC, &["visibility", "nothing"]).unwrap(), DOC);
    }
    #[test]
    fn adds_removes_and_rejects() {
        let out = set(DOC, &["game_password"], &json!("pw")).unwrap();
        assert_eq!(parse(&out).unwrap().as_object().unwrap().keys().next_back().unwrap(), "game_password");
        let out = unset(DOC, &["_comment"]).unwrap();
        assert!(!out.contains("_comment") && out.contains("\"zeta\""));
        assert!(set(DOC, &["name", "x"], &json!(1)).is_err());
        assert!(set("{", &["a"], &json!(1)).is_err() && set("[1]", &["a"], &json!(1)).is_err());
        assert_eq!(set("{}", &["a", "b"], &json!(1)).unwrap().replace(['\n', ' '], ""), "{\"a\":{\"b\":1}}");
    }
}
