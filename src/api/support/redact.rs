//! Secret redaction for support bundles. Pure text-in/text-out: tokens, JWTs, API keys, webhook URLs and long
//! hex/base64-looking strings become `[redacted]`; e-mail addresses are replaced by a stable short hash so that
//! two lines about the same person can still be correlated.
use regex::{Captures, Regex};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Human-readable list of what is removed (shown by the bundle preview).
pub const REDACTIONS: &[&str] = &[
    "JWT access tokens", "API tokens (moss_pat_…) and Bearer credentials", "passwords, secrets and API keys in key=value form",
    "webhook URLs", "backup destination access keys and secrets", "long hexadecimal and base64-like secrets", "e-mail addresses (replaced by a short hash)",
];

struct Rules { webhook: Regex, jwt: Regex, pat: Regex, bearer: Regex, kv: Regex, email: Regex, hex: Regex, b64: Regex }

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| Rules {
        webhook: Regex::new(r#"(?i)https?://[^\s"'<>]*(?:/api/webhooks/|hooks\.slack\.com|/webhooks?/)[^\s"'<>]*"#).unwrap(),
        jwt: Regex::new(r"eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*").unwrap(),
        pat: Regex::new(r"moss_pat_[A-Za-z0-9_-]+").unwrap(),
        bearer: Regex::new(r#"(?i)((?:bearer|basic)\s+)[^\s"']+"#).unwrap(),
        kv: Regex::new(r#"(?i)\b((?:password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|rcon\.password)["']?\s*[:=]\s*["']?)[^\s"',;}&]+"#).unwrap(),
        email: Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)+").unwrap(),
        hex: Regex::new(r"\b[0-9a-fA-F]{32,}\b").unwrap(),
        b64: Regex::new(r"[A-Za-z0-9+/_\-]{40,}={0,2}").unwrap(),
    })
}

/// Stable, non-reversible stand-in for an e-mail address.
pub fn hash_email(e: &str) -> String {
    let d = Sha256::digest(e.trim().to_lowercase().as_bytes());
    format!("<email:{}>", hex::encode(&d[..6]))
}

/// A long run of base64-ish characters is only treated as a secret when it mixes letters and digits and is not
/// shaped like a file path.
fn looks_secret(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit()) && s.chars().any(|c| c.is_ascii_alphabetic()) && s.matches('/').count() <= 2
}

/// Redact secrets in free text. `literals` are exact secrets known to the caller (for example the configured webhook URL).
pub fn redact_text(input: &str, literals: &[&str]) -> String {
    let r = rules();
    let mut s = input.to_string();
    for l in literals.iter().filter(|l| l.len() >= 8) { s = s.replace(l, "[redacted]"); }
    for re in [&r.webhook, &r.jwt, &r.pat] { s = re.replace_all(&s, "[redacted]").into_owned(); }
    s = r.bearer.replace_all(&s, "${1}[redacted]").into_owned();
    s = r.kv.replace_all(&s, "${1}[redacted]").into_owned();
    s = r.email.replace_all(&s, |c: &Captures| hash_email(&c[0])).into_owned();
    s = r.hex.replace_all(&s, "[redacted]").into_owned();
    r.b64.replace_all(&s, |c: &Captures| if looks_secret(&c[0]) { "[redacted]".to_string() } else { c[0].to_string() }).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwts_and_moss_tokens_are_removed() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let out = redact_text(&format!("auth failed for {jwt} here, key moss_pat_AbC123_xyz-9"), &[]);
        assert_eq!(out, "auth failed for [redacted] here, key [redacted]");
        assert_eq!(redact_text("Authorization: Bearer abc.def.ghi", &[]), "Authorization: Bearer [redacted]");
    }

    #[test]
    fn webhook_urls_and_literals_are_removed() {
        let out = redact_text("posting to https://discord.com/api/webhooks/123456/AbCdEf-secret failed: timeout", &[]);
        assert_eq!(out, "posting to [redacted] failed: timeout");
        assert_eq!(redact_text("see https://hooks.slack.com/services/T0/B0/xyz ok", &[]), "see [redacted] ok");
        assert_eq!(redact_text("url is https://example.org/custom-endpoint-1", &["https://example.org/custom-endpoint-1"]), "url is [redacted]");
    }

    #[test]
    fn long_hex_and_base64_secrets_are_removed_but_prose_paths_and_uuids_stay() {
        let hexs = "a".repeat(10) + &"0123456789abcdef".repeat(4);
        assert_eq!(redact_text(&format!("sha256 {hexs} done"), &[]), "sha256 [redacted] done");
        assert_eq!(redact_text("key=Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWprbG1ub3A=", &[]), "key=[redacted]");
        assert_eq!(redact_text("secret: Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWprbG1ub3A", &[]), "secret: [redacted]");
        let keep = "Done (3.2s)! uuid 0195a8e4-7b2c-7d3e-9f10-a1b2c3d4e5f6 path /home/ashton/.config/Claude/scratch-workspaces/abc123/def456/worktree";
        assert_eq!(redact_text(keep, &[]), keep);
        assert_eq!(redact_text("short abc123def456 stays", &[]), "short abc123def456 stays");
    }

    #[test]
    fn key_value_secrets_keep_the_key() {
        assert_eq!(redact_text("rcon.password=hunter2 port=25575", &[]), "rcon.password=[redacted] port=25575");
        assert_eq!(redact_text(r#"{"token": "abc123", "name": "x"}"#, &[]), r#"{"token": "[redacted]", "name": "x"}"#);
        assert_eq!(redact_text("password: s3cr3t!", &[]), "password: [redacted]");
    }

    #[test]
    fn emails_are_hashed_stably_and_case_insensitively() {
        let a = redact_text("signed in: Ashton@Example.com", &[]);
        let b = redact_text("signed in: ashton@example.com", &[]);
        assert_eq!(a, b);
        assert!(a.starts_with("signed in: <email:") && !a.contains("example"));
        assert_ne!(a, redact_text("signed in: other@example.com", &[]));
        assert_eq!(hash_email("x@y.io").len(), "<email:>".len() + 12);
    }
}
