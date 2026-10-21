//! Image selection and strict validation of image references. Only references from an allow-list of registries
//! and repositories are ever passed to the engine, always fully qualified and never starting with `-`.
use crate::server::Srv;

/// Repositories (fully qualified) a server may use, plus any repository under these registries.
const REPOS: &[&str] = &["docker.io/library/eclipse-temurin", "docker.io/azul/zulu-openjdk", "docker.io/library/amazoncorretto",
    "docker.io/library/debian", "docker.io/library/ubuntu", "docker.io/steamcmd/steamcmd", "docker.io/itzg/minecraft-bedrock-server"];
const REGISTRIES: &[&str] = &["ghcr.io"];

/// The default image for a Java major version.
pub fn default_for(java_major: i64) -> String { format!("docker.io/library/eclipse-temurin:{}-jre", java_major.clamp(8, 99)) }

/// The default image for a game. Games run their own installed binary (the server folder is mounted), so the image only
/// has to carry the shared libraries it needs; all of these are published for linux/amd64.
/// - Bedrock needs libcurl and libssl, which the plain Debian/Ubuntu images lack: the Bedrock community image has them.
/// - Steam (srcds, Valheim, Palworld, ...) needs 32-bit glibc/libstdc++ and CA certificates: the SteamCMD Ubuntu 24.04 image.
/// - Terraria and Factorio ship their own libraries and only need glibc: slim Debian (small).
pub fn for_game(id: &str, steam: bool) -> &'static str {
    match (id, steam) {
        ("bedrock", _) => "docker.io/itzg/minecraft-bedrock-server:latest",
        (_, true) => "docker.io/steamcmd/steamcmd:ubuntu-24",
        _ => "docker.io/library/debian:bookworm-slim",
    }
}

/// The image a server runs: its (validated) override, else the default for its game, else for its Java version.
pub fn for_server(s: &Srv) -> String {
    match s.game() {
        Some(g) => s.container_image.as_deref().and_then(|o| validate(o).ok()).unwrap_or_else(|| for_game(g.id, matches!(g.install, crate::games::Install::Steam(_))).to_string()),
        None => for_java(s.container_image.as_deref(), s.java_major),
    }
}

pub fn for_java(over: Option<&str>, java_major: i64) -> String {
    over.and_then(|o| validate(o).ok()).unwrap_or_else(|| default_for(java_major))
}

fn component_ok(c: &str) -> bool {
    !c.is_empty() && c.len() <= 64 && c.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
        && c.starts_with(|ch: char| ch.is_ascii_alphanumeric()) && c.ends_with(|ch: char| ch.is_ascii_alphanumeric())
}

fn tag_ok(t: &str) -> bool {
    !t.is_empty() && t.len() <= 128 && t.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') && t.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// Validate and normalise (fully qualify) an image reference. A tag or a sha256 digest is required.
pub fn validate(raw: &str) -> Result<String, String> {
    let r = raw.trim();
    if r.is_empty() || r.len() > 255 { return Err("enter an image such as docker.io/library/eclipse-temurin:21-jre".into()); }
    let (name_tag, digest) = match r.split_once('@') {
        Some((n, d)) => {
            let h = d.strip_prefix("sha256:").ok_or("the digest must be sha256:<64 hex digits>")?;
            if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { return Err("the digest must be sha256:<64 hex digits>".into()); }
            (n, Some(d))
        }
        None => (r, None),
    };
    // a ':' after the last '/' separates the tag (a ':' before it would be a registry port, which is not allowed)
    let (path, tag) = match name_tag.rsplit_once(':') {
        Some((p, t)) if !t.contains('/') => (p, Some(t)),
        _ => (name_tag, None),
    };
    if tag.is_none() && digest.is_none() { return Err("the image needs a tag (for example :21-jre) or a digest".into()); }
    if tag.is_some_and(|t| !tag_ok(t)) { return Err("the image tag contains characters that are not allowed".into()); }
    let parts: Vec<&str> = path.split('/').collect();
    let full: Vec<&str> = if parts.len() == 1 { vec!["docker.io", "library", parts[0]] }
        else if parts[0].contains('.') { parts }
        else { let mut v = vec!["docker.io"]; v.extend(parts); v };
    if full.len() < 3 || !full[1..].iter().all(|c| component_ok(c)) || !full[0].bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-')) {
        return Err("that is not a valid image reference".into());
    }
    let canon_path = full.join("/");
    let allowed = REPOS.contains(&canon_path.as_str()) || REGISTRIES.contains(&full[0]);
    if !allowed { return Err(format!("images are only allowed from {} and {}", REPOS.join(", "), REGISTRIES.iter().map(|r| format!("{r}/…")).collect::<Vec<_>>().join(", "))); }
    Ok(format!("{canon_path}{}{}", tag.map(|t| format!(":{t}")).unwrap_or_default(), digest.map(|d| format!("@{d}")).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_temurin_jre_for_the_java_major() {
        assert_eq!(default_for(21), "docker.io/library/eclipse-temurin:21-jre");
        assert_eq!(default_for(17), "docker.io/library/eclipse-temurin:17-jre");
    }

    #[test]
    fn games_get_their_own_default_image_and_overrides_still_work() {
        use crate::runtimes::launch::test_srv;
        let mut s = test_srv("bedrock", "x", None, "balanced");
        assert_eq!(for_server(&s), "docker.io/itzg/minecraft-bedrock-server:latest");
        for (g, want) in [("terraria", "docker.io/library/debian:bookworm-slim"), ("factorio", "docker.io/library/debian:bookworm-slim"), ("valheim", "docker.io/steamcmd/steamcmd:ubuntu-24"), ("cs2", "docker.io/steamcmd/steamcmd:ubuntu-24")] {
            s.runtime = g.into();
            assert_eq!(for_server(&s), want, "{g}");
            assert_eq!(validate(want).unwrap(), want, "{g} default is on the allow-list");
        }
        s.container_image = Some("ubuntu:24.04".into());
        assert_eq!(for_server(&s), "docker.io/library/ubuntu:24.04");
        s.container_image = Some("evil/x:1".into());
        assert_eq!(for_server(&s), "docker.io/steamcmd/steamcmd:ubuntu-24", "a bad stored override falls back");
        let j = test_srv("paper", "1.21.1", None, "minimal");
        assert_eq!(for_server(&j), default_for(j.java_major));
    }

    #[test]
    fn accepts_allow_listed_references_and_qualifies_them() {
        assert_eq!(validate("eclipse-temurin:21-jre").unwrap(), "docker.io/library/eclipse-temurin:21-jre");
        assert_eq!(validate("docker.io/library/eclipse-temurin:17-jre-alpine").unwrap(), "docker.io/library/eclipse-temurin:17-jre-alpine");
        assert_eq!(validate("ghcr.io/someone/mc-java:21").unwrap(), "ghcr.io/someone/mc-java:21");
        let d = format!("ghcr.io/o/i@sha256:{}", "a".repeat(64));
        assert_eq!(validate(&d).unwrap(), d);
        assert!(validate("azul/zulu-openjdk:21-jre").is_ok());
    }

    #[test]
    fn rejects_everything_else() {
        for bad in ["", "eclipse-temurin", "evil.example.com/x/y:1", "docker.io/someone/else:1", "alpine:3.20", "docker.io/library/debian", "-v/:/host:1", "ghcr.io/o/i:1 --privileged",
            "ghcr.io/o/i:1;rm", "ghcr.io/O/I:1", "ghcr.io/o/../i:1", "localhost:5000/x/y:1", "ghcr.io/o/i@sha256:abc", "ghcr.io/o/i@md5:aa", "ghcr.io//i:1", "ghcr.io/o/i:\n1",
            "eclipse-temurin:-1", "ghcr.io:443/o/i:1", "http://ghcr.io/o/i:1", "ghcr.io/o/i:1\0"] {
            assert!(validate(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn a_bad_stored_override_falls_back_to_the_default() {
        assert_eq!(for_java(Some("evil/x:1"), 21), default_for(21));
        assert_eq!(for_java(None, 8), "docker.io/library/eclipse-temurin:8-jre");
    }
}
