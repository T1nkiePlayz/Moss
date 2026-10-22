// Copies the shipped sign-in settings (the repository's `.env`: the project's Supabase URL and its public
// anon key) into the build, so an installed binary can sign people in without any config file next to it.
// See `dotenv::shipped_signin`. A checkout without `.env` builds with no shipped settings.
fn main() {
    // Watching a missing file would rerun this script on every build; without it Cargo's default (any package file) applies.
    let text = std::fs::read_to_string(".env").ok();
    if text.is_some() { println!("cargo:rerun-if-changed=.env"); }
    let text = text.unwrap_or_default();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("shipped.env");
    std::fs::write(out, text).unwrap();
}
