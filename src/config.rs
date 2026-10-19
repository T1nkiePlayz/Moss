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
    /// Expose the simulated game server runtime (development and demos).
    #[arg(long, env = "MOSS_DEV_RUNTIMES", default_value_t = false, action = clap::ArgAction::Set)]
    pub dev_runtimes: bool,
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

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.dev() {
            let ip: std::net::IpAddr = self.bind.parse().map_err(|_| anyhow::anyhow!("--bind must be an IP address"))?;
            if !ip.is_loopback() {
                anyhow::bail!("dev auth mode (no login) may only bind to loopback; refusing to bind {}", self.bind);
            }
        } else if self.supabase_url.is_empty() {
            anyhow::bail!("SUPABASE_URL is required in supabase auth mode (or run with MOSS_AUTH=dev on loopback)");
        }
        let u = &self.curseforge_api_url;
        if !(u.starts_with("https://") || u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost")) {
            anyhow::bail!("MOSS_CURSEFORGE_API_URL must be https (or loopback http for tests)");
        }
        Ok(())
    }
}
