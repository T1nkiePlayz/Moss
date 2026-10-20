//! MFA policy (`MOSS_REQUIRE_MFA`): who must present an `aal2` session, and the 403 body when they do not.
use crate::config::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Off, Admins, All }

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_lowercase().as_str() { "" | "off" => Some(Mode::Off), "admins" => Some(Mode::Admins), "all" => Some(Mode::All), _ => None }
    }
}

impl Config {
    /// The configured policy; `validate` rejects unknown values, so a bad value here falls back to the strictest.
    pub fn mfa_mode(&self) -> Mode { Mode::parse(&self.require_mfa).unwrap_or(Mode::All) }
}

/// Is MFA required for an account with this role under `mode`?
pub fn required(mode: Mode, role: &str) -> bool {
    match mode { Mode::Off => false, Mode::All => true, Mode::Admins => role == "owner" || role == "admin" }
}

pub fn aal2(aal: Option<&str>) -> bool { aal == Some("aal2") }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_matrix() {
        for (mode, role, want) in [
            (Mode::Off, "owner", false), (Mode::Off, "user", false),
            (Mode::Admins, "owner", true), (Mode::Admins, "admin", true), (Mode::Admins, "user", false),
            (Mode::All, "owner", true), (Mode::All, "user", true),
        ] { assert_eq!(required(mode, role), want, "{mode:?} {role}"); }
    }
    #[test]
    fn aal_levels() {
        assert!(aal2(Some("aal2")));
        assert!(!aal2(Some("aal1")) && !aal2(None) && !aal2(Some("")));
    }
    #[test]
    fn parses_modes() {
        assert_eq!(Mode::parse("OFF"), Some(Mode::Off));
        assert_eq!(Mode::parse(" admins "), Some(Mode::Admins));
        assert_eq!(Mode::parse("all"), Some(Mode::All));
        assert_eq!(Mode::parse("sometimes"), None);
    }
}
