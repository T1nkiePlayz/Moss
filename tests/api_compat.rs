//! API stability contract (docs/api-policy.md): the current docs/openapi.json must stay compatible with the committed
//! snapshot docs/openapi.frozen.json. Additive changes pass. A deliberate breaking change needs a new API version, or
//! (before 1.0 ships it) a conscious refresh of the snapshot with `node scripts/api-freeze.mjs`.
use serde_json::Value;
use std::collections::BTreeSet;

const MAX_DEPTH: usize = 12;

fn resolve<'a>(spec: &'a Value, v: &'a Value) -> &'a Value {
    let mut v = v;
    for _ in 0..8 {
        match v.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix('#')) { Some(p) => v = spec.pointer(p).unwrap_or(&Value::Null), None => break }
    }
    v
}

fn types(v: &Value) -> Option<BTreeSet<String>> {
    match &v["type"] { Value::String(s) => Some([s.clone()].into()), Value::Array(a) => Some(a.iter().filter_map(|x| x.as_str().map(String::from)).collect()), _ => None }
}
/// Is every value of type set `a` also acceptable as one of `b` (integers are numbers)?
fn within(a: &BTreeSet<String>, b: &BTreeSet<String>) -> bool { a.iter().all(|t| b.contains(t) || (t == "integer" && b.contains("number"))) }
fn enum_of(v: &Value) -> Option<BTreeSet<String>> { v["enum"].as_array().map(|a| a.iter().map(|x| x.to_string()).collect()) }

#[derive(Clone, Copy, PartialEq)]
enum Dir { Response, Request }

/// Compare an old schema with a new one in the given direction; problems are appended to `errs`.
fn schema(old_spec: &Value, old: &Value, new_spec: &Value, new: &Value, dir: Dir, at: &str, depth: usize, errs: &mut Vec<String>) {
    if depth > MAX_DEPTH { return; }
    let (o, n) = (resolve(old_spec, old), resolve(new_spec, new));
    if let Some(ot) = types(o) {
        match (types(n), dir) {
            (None, Dir::Response) => errs.push(format!("{at}: type {ot:?} is no longer declared")),
            (Some(nt), Dir::Response) if !within(&nt, &ot) => errs.push(format!("{at}: response type changed from {ot:?} to {nt:?}")),
            (Some(nt), Dir::Request) if !within(&ot, &nt) => errs.push(format!("{at}: request type changed from {ot:?} to {nt:?} (values that were accepted are now refused)")),
            _ => {}
        }
    }
    if let (Some(oe), Some(ne)) = (enum_of(o), enum_of(n)) {
        let gone: Vec<_> = oe.difference(&ne).collect();
        if !gone.is_empty() { errs.push(format!("{at}: enum value(s) removed: {gone:?}")); }
    }
    if dir == Dir::Request {
        let req = |v: &Value| -> BTreeSet<String> { v["required"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
        for k in req(n).difference(&req(o)) { errs.push(format!("{at}: request field {k} is newly required")); }
    }
    if let Some(op) = o["properties"].as_object() {
        for (k, ov) in op {
            match n["properties"].get(k) {
                None => errs.push(format!("{at}.{k}: {} field removed", if dir == Dir::Response { "response" } else { "request" })),
                Some(nv) => schema(old_spec, ov, new_spec, nv, dir, &format!("{at}.{k}"), depth + 1, errs),
            }
        }
    }
    if o.get("items").is_some() && n.get("items").is_some() { schema(old_spec, &o["items"], new_spec, &n["items"], dir, &format!("{at}[]"), depth + 1, errs); }
}

fn json_schema<'a>(op: &'a Value, code: &str) -> Option<&'a Value> { op["responses"][code]["content"]["application/json"].get("schema") }

/// Every way `new` breaks the contract that `old` promised. Empty = compatible.
pub fn breaking_changes(old: &Value, new: &Value) -> Vec<String> {
    let mut errs = vec![];
    let none = serde_json::Map::new();
    for (path, oitem) in old["paths"].as_object().unwrap_or(&none) {
        for (method, oop) in oitem.as_object().unwrap_or(&none) {
            let at = format!("{} {path}", method.to_uppercase());
            let Some(nop) = new["paths"][path].get(method) else { errs.push(format!("{at}: removed")); continue };
            for (code, _) in oop["responses"].as_object().unwrap_or(&none).iter().filter(|(c, _)| c.starts_with('2')) {
                if nop["responses"].get(code).is_none() { errs.push(format!("{at}: success status {code} is gone")); continue; }
                if let (Some(os), Some(ns)) = (json_schema(oop, code), json_schema(nop, code)) { schema(old, os, new, ns, Dir::Response, &format!("{at} -> {code}"), 0, &mut errs); }
            }
            if let (Some(os), Some(ns)) = (oop["requestBody"]["content"]["application/json"].get("schema"), nop["requestBody"]["content"]["application/json"].get("schema")) {
                schema(old, os, new, ns, Dir::Request, &format!("{at} body"), 0, &mut errs);
            }
            if oop["requestBody"]["content"]["application/json"].get("schema").is_some() && nop["requestBody"].is_null() { errs.push(format!("{at}: request body no longer accepted")); }
            let (op_, np) = (oop["parameters"].as_array().cloned().unwrap_or_default(), nop["parameters"].as_array().cloned().unwrap_or_default());
            let key = |p: &Value| (p["name"].as_str().unwrap_or("").to_string(), p["in"].as_str().unwrap_or("").to_string());
            for p in &op_ { if !np.iter().any(|q| key(q) == key(p)) { errs.push(format!("{at}: parameter {:?} removed", key(p))); } }
            for q in &np {
                let existed = op_.iter().find(|p| key(p) == key(q));
                if q["required"] == true && existed.is_none_or(|p| p["required"] != true) && q["in"] != "path" { errs.push(format!("{at}: parameter {:?} is newly required", key(q))); }
                if let Some(p) = existed && let (Some(os), Some(ns)) = (p.get("schema"), q.get("schema")) { schema(old, os, new, ns, Dir::Request, &format!("{at} parameter {}", key(q).0), 0, &mut errs); }
            }
        }
    }
    errs
}

fn load(name: &str) -> Value { serde_json::from_str(&std::fs::read_to_string(format!("{}/docs/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap_or_else(|e| panic!("{name}: {e}"))).unwrap() }

#[test]
fn current_api_is_compatible_with_the_frozen_snapshot() {
    let (frozen, current) = (load("openapi.frozen.json"), load("openapi.json"));
    assert!(frozen["paths"].as_object().is_some_and(|p| p.len() > 80), "the frozen snapshot looks empty");
    let errs = breaking_changes(&frozen, &current);
    assert!(errs.is_empty(), "breaking API changes relative to docs/openapi.frozen.json (see docs/api-policy.md):\n  {}", errs.join("\n  "));
}

#[cfg(test)]
mod self_tests {
    use super::*;
    use serde_json::json;

    fn spec(get_schema: Value, post_schema: Value, extra_param: bool) -> Value {
        let mut params = vec![json!({"name": "id", "in": "path", "required": true, "schema": {"type": "string"}})];
        if extra_param { params.push(json!({"name": "q", "in": "query", "required": true, "schema": {"type": "string"}})); }
        json!({"paths": {"/things/{id}": {
            "get": {"parameters": params, "responses": {"200": {"description": "OK", "content": {"application/json": {"schema": get_schema}}}}},
            "post": {"requestBody": {"content": {"application/json": {"schema": post_schema}}}, "responses": {"201": {"description": "c"}}}}},
          "components": {"schemas": {"Thing": {"type": "object", "properties": {"id": {"type": "string"}, "state": {"type": "string", "enum": ["a", "b"]}, "n": {"type": "integer"}}}}}})
    }
    fn base() -> Value { spec(json!({"$ref": "#/components/schemas/Thing"}), json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}), false) }

    #[test]
    fn additive_changes_pass() {
        let mut new = base();
        new["components"]["schemas"]["Thing"]["properties"]["extra"] = json!({"type": "string"});
        new["components"]["schemas"]["Thing"]["properties"]["state"]["enum"] = json!(["a", "b", "c"]);
        new["paths"]["/new"] = json!({"get": {"responses": {"200": {"description": "OK"}}}});
        new["paths"]["/things/{id}"]["post"]["requestBody"]["content"]["application/json"]["schema"]["properties"]["opt"] = json!({"type": "string"});
        new["paths"]["/things/{id}"]["get"]["parameters"].as_array_mut().unwrap().push(json!({"name": "limit", "in": "query", "required": false, "schema": {"type": "integer"}}));
        assert_eq!(breaking_changes(&base(), &new), Vec::<String>::new());
    }

    #[test]
    fn each_kind_of_break_is_caught() {
        let mut m = base(); m["paths"].as_object_mut().unwrap().remove("/things/{id}");
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains("GET /things/{id}: removed")));
        let mut m = base(); m["paths"]["/things/{id}"].as_object_mut().unwrap().remove("post");
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains("POST /things/{id}: removed")));
        let mut m = base(); m["components"]["schemas"]["Thing"]["properties"].as_object_mut().unwrap().remove("n");
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains(".n: response field removed")));
        let mut m = base(); m["components"]["schemas"]["Thing"]["properties"]["n"]["type"] = json!("string");
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains("response type changed")));
        let mut m = base(); m["components"]["schemas"]["Thing"]["properties"]["state"]["enum"] = json!(["a"]);
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains("enum value(s) removed")));
        let mut m = base(); m["paths"]["/things/{id}"]["post"]["requestBody"]["content"]["application/json"]["schema"]["properties"]["more"] = json!({"type": "string"});
        m["paths"]["/things/{id}"]["post"]["requestBody"]["content"]["application/json"]["schema"]["required"] = json!(["name", "more"]);
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains("newly required")));
        let m = spec(json!({"$ref": "#/components/schemas/Thing"}), base()["paths"]["/things/{id}"]["post"]["requestBody"]["content"]["application/json"]["schema"].clone(), true);
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains("parameter") && e.contains("newly required")));
        let mut m = base(); m["paths"]["/things/{id}"]["get"]["responses"] = json!({"204": {"description": "none"}});
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains("success status 200 is gone")));
        let mut m = base(); m["paths"]["/things/{id}"]["post"]["requestBody"]["content"]["application/json"]["schema"]["properties"]["name"] = json!({"type": "integer"});
        assert!(breaking_changes(&base(), &m).iter().any(|e| e.contains("request type changed")));
    }

    #[test]
    fn a_request_field_that_becomes_optional_or_a_type_that_narrows_in_a_response_is_fine() {
        let mut m = base();
        m["paths"]["/things/{id}"]["post"]["requestBody"]["content"]["application/json"]["schema"]["required"] = json!([]);
        m["components"]["schemas"]["Thing"]["properties"]["id"]["type"] = json!("string");
        assert!(breaking_changes(&base(), &m).is_empty());
        let (mut old, mut new) = (base(), base());
        old["components"]["schemas"]["Thing"]["properties"]["n"]["type"] = json!(["integer", "null"]);
        new["components"]["schemas"]["Thing"]["properties"]["n"]["type"] = json!("integer");
        assert!(breaking_changes(&old, &new).is_empty());
        assert!(!breaking_changes(&new, &old).is_empty(), "response gaining null is a break");
    }
}
