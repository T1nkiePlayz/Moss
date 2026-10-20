//! The GitHub workflows must be valid YAML, and every third-party action must be pinned to a full commit SHA.
use serde_yaml_ng::Value;

fn walk(v: &Value, f: &mut dyn FnMut(&str)) {
    match v {
        Value::Mapping(m) => for (k, v) in m { if k.as_str() == Some("uses") && let Some(s) = v.as_str() { f(s); } walk(v, f); },
        Value::Sequence(s) => s.iter().for_each(|v| walk(v, f)),
        _ => {}
    }
}

#[test]
fn workflows_parse_and_pin_actions() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut seen = 0;
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if !matches!(p.extension().and_then(|x| x.to_str()), Some("yml" | "yaml")) { continue }
        let v: Value = serde_yaml_ng::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(v.get("jobs").is_some(), "{} has no jobs", p.display());
        walk(&v, &mut |u| {
            if u.starts_with("./") { return }
            let (_, r) = u.split_once('@').unwrap_or_else(|| panic!("{}: `{u}` has no version", p.display()));
            assert!(r.len() == 40 && r.chars().all(|c| c.is_ascii_hexdigit()), "{}: `{u}` is not pinned to a commit SHA", p.display());
        });
        seen += 1;
    }
    assert!(seen >= 3, "expected the ci, release and soak workflows");
}
