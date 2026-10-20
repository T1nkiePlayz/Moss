//! Keeps docs/openapi.json honest: every route in the router is documented and nothing documented is missing.
use std::collections::BTreeSet;

fn routes_from_source() -> BTreeSet<(String, String)> {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/api/mod.rs")).unwrap();
    let mut out = BTreeSet::new();
    for line in src.lines().filter(|l| l.contains(".route(\"")) {
        let rest = line.split(".route(\"").nth(1).unwrap();
        let (path, verbs) = rest.split_once('"').unwrap();
        // WebSocket upgrades and SSE are GET like any other route
        if path == "/healthz" { continue; } // lives outside /api/v1; documented in docs/api.md
        for v in ["get", "post", "put", "patch", "delete"] {
            let call = if v == "delete" { ["delete(", "routing::delete("] } else { [&*format!("{v}("), &*format!("{v}(")] };
            let found = verbs.split(|c: char| !c.is_alphanumeric() && c != ':' && c != '_' && c != '(').any(|tok| call.iter().any(|c| tok.starts_with(c)));
            if found { out.insert((path.to_string(), v.to_string())); }
        }
    }
    out
}

fn routes_from_spec() -> BTreeSet<(String, String)> {
    let spec: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/openapi.json")).unwrap()).unwrap();
    let mut out = BTreeSet::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for m in item.as_object().unwrap().keys() { out.insert((path.clone(), m.clone())); }
    }
    out
}

#[test]
fn every_route_is_documented_and_every_documented_route_exists() {
    let (code, spec) = (routes_from_source(), routes_from_spec());
    assert!(code.len() > 80, "route scan found only {} routes", code.len());
    let undocumented: Vec<_> = code.difference(&spec).collect();
    let phantom: Vec<_> = spec.difference(&code).collect();
    assert!(undocumented.is_empty(), "routes missing from docs/openapi.json: {undocumented:?}");
    assert!(phantom.is_empty(), "documented routes that do not exist: {phantom:?}");
}
