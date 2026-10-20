//! `.env` / `.env.local` loading: pure parsing (`parse`) plus the process-environment side effect (`load`).
use std::collections::{HashMap, HashSet};

/// Parse `KEY=value` lines (`#` comments, optional `export `, optional quotes), skipping empty values and any key in `preset`.
pub fn parse(text: &str, preset: &HashSet<String>) -> Vec<(String, String)> {
    let mut out = vec![];
    for line in text.lines() {
        let line = line.trim();
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim().trim_start_matches("export ").trim(), v.trim().trim_matches(|c| c == '"' || c == '\''));
        if k.is_empty() || k.starts_with('#') || v.is_empty() || preset.contains(k) { continue; }
        out.push((k.to_string(), v.to_string()));
    }
    out
}

/// Load `.env` then `.env.local` from the working directory (and the binary's directory). Real environment
/// variables always win, then `.env.local`, then `.env`. Lines are `KEY=value`; `#` starts a comment.
pub fn load() {
    let mut dirs = vec![std::path::PathBuf::from(".")];
    if let Some(d) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) { dirs.push(d); }
    let preset: HashSet<String> = std::env::vars().map(|(k, _)| k).collect();
    let mut loaded: HashMap<String, String> = Default::default();
    for name in [".env", ".env.local"] {
        for d in &dirs {
            let Ok(text) = std::fs::read_to_string(d.join(name)) else { continue };
            loaded.extend(parse(&text, &preset));
        }
    }
    // single-threaded at this point, before the runtime starts
    for (k, v) in loaded { unsafe { std::env::set_var(k, v); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_pairs() {
        let preset: HashSet<String> = ["SET".to_string()].into();
        let t = "# c\nA=1\nexport B = \"two\"\nC='3'\nEMPTY=\nSET=x\nnoequals\n=v\n";
        assert_eq!(parse(t, &preset), vec![("A".into(), "1".into()), ("B".into(), "two".into()), ("C".into(), "3".into())]);
    }
}
