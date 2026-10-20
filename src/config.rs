use clap::Args;
use std::path::PathBuf;

#[derive(Args, Clone, Debug)]
pub struct Config {
    /// Address to bind. Loopback by default; remote exposure is a deliberate choice.
    #[arg(long, env = "MOSS_BIND", default_value = "127.0.0.1")]
    pub bind: String,
    #[arg(long, env = "MOSS_PORT", default_value_t = 8420)]
    pub port: u16,
    /// Data directory (database, servers, backups, cache).
    #[arg(long, env = "MOSS_DATA_DIR", default_value = "./moss-data")]
    pub data_dir: PathBuf,
    /// `supabase` (production) or `dev` (no login, loopback only).
    #[arg(long, env = "MOSS_AUTH", default_value = "supabase")]
    pub auth: String,
    #[arg(long, env = "SUPABASE_URL", default_value = "")]
    pub supabase_url: String,
    /// Public (publishable/anon) key, handed to the browser.
    #[arg(long, env = "SUPABASE_ANON_KEY", default_value = "")]
    pub supabase_anon_key: String,
    /// Legacy HS256 JWT secret. Not needed for projects using asymmetric signing keys (JWKS).
    #[arg(long, env = "SUPABASE_JWT_SECRET", default_value = "", hide_env_values = true)]
    pub supabase_jwt_secret: String,
    /// Comma separated emails that become owner on first sign-in.
    #[arg(long, env = "MOSS_OWNER_EMAILS", default_value = "")]
    pub owner_emails: String,
    /// Require Supabase `email_verified` before an account can be matched to an owner/invite.
    #[arg(long, env = "MOSS_REQUIRE_VERIFIED_EMAIL", default_value_t = true, action = clap::ArgAction::Set)]
    pub require_verified_email: bool,
    /// Your own CurseForge API key for direct access (secret; keep it in .env.local). Without it, Moss uses the project's `curseforge` edge function.
    #[arg(long, env = "CURSEFORGE_API_KEY", default_value = "", hide_env_values = true)]
    pub curseforge_api_key: String,
    /// API base for direct CurseForge access (https, or loopback http for tests).
    #[arg(long, env = "MOSS_CURSEFORGE_API_URL", default_value = "https://api.curseforge.com")]
    pub curseforge_api_url: String,
    /// Edge function URL for CurseForge; empty = `<SUPABASE_URL>/functions/v1/curseforge`, `off` disables CurseForge.
    #[arg(long, env = "MOSS_CURSEFORGE_PROXY", default_value = "")]
    pub curseforge_proxy: String,
    /// API base for Modrinth (https, or loopback http for tests).
    #[arg(long, env = "MOSS_MODRINTH_API_URL", default_value = "https://api.modrinth.com/v2")]
    pub modrinth_api_url: String,
    /// Ports staged update copies may listen on, `first-last`.
    #[arg(long, env = "MOSS_STAGING_PORT_RANGE", default_value = "30000-30999")]
    pub staging_port_range: String,
    /// Allow notification webhooks that resolve to private, loopback or link-local addresses (default: refuse, to prevent SSRF).
    #[arg(long, env = "MOSS_ALLOW_PRIVATE_WEBHOOKS", default_value_t = false, action = clap::ArgAction::Set)]
    pub allow_private_webhooks: bool,
    /// Allow remote backup destinations (S3 endpoints) that resolve to private or loopback addresses, such as a MinIO on your LAN (default: refuse, to prevent SSRF).
    #[arg(long, env = "MOSS_ALLOW_PRIVATE_BACKUP_HOSTS", default_value_t = false, action = clap::ArgAction::Set)]
    pub allow_private_backup_hosts: bool,
    /// MFA policy: `off`, `admins` (owner/admin accounts need an aal2 session) or `all`.
    #[arg(long, env = "MOSS_REQUIRE_MFA", default_value = "off")]
    pub require_mfa: String,
    /// Trust `X-Forwarded-For` for the client address (only behind a reverse proxy that sets it).
    #[arg(long, env = "MOSS_TRUST_PROXY", default_value_t = false, action = clap::ArgAction::Set)]
    pub trust_proxy: bool,
    /// Expose the simulated game server runtime (development and demos).
    #[arg(long, env = "MOSS_DEV_RUNTIMES", default_value_t = false, action = clap::ArgAction::Set)]
    pub dev_runtimes: bool,
    /// Container engine CLI for servers set to container isolation: a name (`docker`, `podman`) or a path. Empty = detect (rootless Podman, then Docker, then Podman).
    #[arg(long, env = "MOSS_CONTAINER_ENGINE", default_value = "")]
    pub container_engine: String,
}

impl Config {
    pub fn dev(&self) -> bool {
        self.auth == "dev"
    }
    pub fn owner_email_list(&self) -> Vec<String> {
        self.owner_emails.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
    }
    pub fn servers_dir(&self) -> PathBuf { self.data_dir.join("servers") }
    pub fn backups_dir(&self) -> PathBuf { self.data_dir.join("backups") }
    pub fn cache_dir(&self) -> PathBuf { self.data_dir.join("cache") }
    pub fn tmp_dir(&self) -> PathBuf { self.data_dir.join("tmp") }
    pub fn java_dir(&self) -> PathBuf { self.data_dir.join("java") }
    pub fn staging_ports(&self) -> (u16, u16) { parse_port_range(&self.staging_port_range).unwrap_or((30000, 30999)) }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.dev() {
            let ip: std::net::IpAddr = self.bind.parse().map_err(|_| anyhow::anyhow!("--bind must be an IP address"))?;
            if !ip.is_loopback() {
                anyhow::bail!("dev auth mode (no login) may only bind to loopback; refusing to bind {}", self.bind);
            }
        } else if self.supabase_url.is_empty() {
            anyhow::bail!("SUPABASE_URL is required in supabase auth mode (or run with MOSS_AUTH=dev on loopback)");
        }
        if crate::auth::MfaMode::parse(&self.require_mfa).is_none() { anyhow::bail!("MOSS_REQUIRE_MFA must be off, admins or all"); }
        let u = &self.curseforge_api_url;
        if !(u.starts_with("https://") || u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost")) {
            anyhow::bail!("MOSS_CURSEFORGE_API_URL must be https (or loopback http for tests)");
        }
        let m = &self.modrinth_api_url;
        if !(m.starts_with("https://") || m.starts_with("http://127.0.0.1") || m.starts_with("http://localhost")) {
            anyhow::bail!("MOSS_MODRINTH_API_URL must be https (or loopback http for tests)");
        }
        if parse_port_range(&self.staging_port_range).is_none() { anyhow::bail!("MOSS_STAGING_PORT_RANGE must look like 30000-30999 (1024 or higher, first <= last)"); }
        Ok(())
    }
}

/// `first-last` with 1024 <= first <= last.
pub fn parse_port_range(s: &str) -> Option<(u16, u16)> {
    let (a, b) = s.trim().split_once('-')?;
    let (a, b): (u16, u16) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a >= 1024 && a <= b).then_some((a, b))
}

/// A configuration for tests: dev auth, the given data directory, everything else default.
#[cfg(test)]
pub fn for_test(dir: &std::path::Path) -> Config {
    use clap::Parser;
    #[derive(Parser)] struct W { #[command(flatten)] c: Config }
    let mut c = W::parse_from(["moss", "--auth", "dev"]).c;
    c.data_dir = dir.to_path_buf();
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn port_ranges_are_validated() {
        assert_eq!(parse_port_range("30000-30999"), Some((30000, 30999)));
        assert_eq!(parse_port_range(" 2000 - 2000 "), Some((2000, 2000)));
        for bad in ["", "30000", "30999-30000", "80-90", "a-b", "1000-99999"] { assert_eq!(parse_port_range(bad), None, "{bad}"); }
    }
}
