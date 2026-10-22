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

/// The repository's `.env` at build time (see build.rs): the sign-in settings releases ship with.
pub const SHIPPED: &str = include_str!(concat!(env!("OUT_DIR"), "/shipped.env"));

/// The lowest layer, under flags, the environment and both files: the shipped Supabase URL and anon key, so an
/// installed binary signs people in with no config file. Used only when neither the URL nor the key was given
/// (an empty value counts as not given), so a URL or key of your own is never mixed with the shipped one.
/// Nothing else is taken from the shipped file.
pub fn fill_shipped_signin(url: &mut String, key: &mut String, shipped: &str) {
    if !url.is_empty() || !key.is_empty() { return; }
    let kv: HashMap<String, String> = parse(shipped, &HashSet::new()).into_iter().collect();
    if let (Some(u), Some(k)) = (kv.get("SUPABASE_URL"), kv.get("SUPABASE_ANON_KEY")) { (*url, *key) = (u.clone(), k.clone()); }
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

    #[test]
    fn shipped_signin_fills_only_an_unconfigured_install() {
        let shipped = "SUPABASE_URL=https://shipped.supabase.co\nSUPABASE_ANON_KEY=shipped-key\nMOSS_PORT=1\n";
        let fill = |u: &str, k: &str, shipped: &str| { let (mut u, mut k) = (u.to_string(), k.to_string()); fill_shipped_signin(&mut u, &mut k, shipped); (u, k) };
        let pair = |u: &str, k: &str| (u.to_string(), k.to_string());
        // nothing configured (or only empty values): both shipped values
        assert_eq!(fill("", "", shipped), pair("https://shipped.supabase.co", "shipped-key"));
        // a URL or a key of your own, from a flag, the environment or a file, is never mixed with a shipped one
        assert_eq!(fill("https://mine.supabase.co", "", shipped), pair("https://mine.supabase.co", ""));
        assert_eq!(fill("", "my-key", shipped), pair("", "my-key"));
        // a build without a complete shipped pair adds nothing
        assert_eq!(fill("", "", ""), pair("", ""));
        assert_eq!(fill("", "", "SUPABASE_URL=https://shipped.supabase.co\n"), pair("", ""));
    }

    #[test]
    fn the_build_ships_the_repository_sign_in_settings() {
        let repo = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).unwrap_or_default();
        assert_eq!(SHIPPED, repo, "build.rs copies .env verbatim");
    }
}
