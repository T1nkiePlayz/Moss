//! Extra published ports (plugins such as Simple Voice Chat or Geyser) and the container network mode.
use crate::server::Srv;
use serde::{Deserialize, Serialize};

pub const MAX_EXTRA: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Proto { Tcp, Udp }
impl Proto { pub fn name(self) -> &'static str { match self { Proto::Tcp => "tcp", Proto::Udp => "udp" } } }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraPort { pub port: u16, pub proto: Proto }

/// Name of the Moss-managed network for `game-only` servers.
pub const GAMEONLY_NET: &str = "moss-gameonly";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network { Open, GameOnly }
impl Network {
    pub fn parse(s: &str) -> Option<Network> { match s { "open" => Some(Network::Open), "game-only" => Some(Network::GameOnly), _ => None } }
}

/// Stored form (never fails: a damaged value means no extra ports).
pub fn parse_stored(s: &str) -> Vec<ExtraPort> { serde_json::from_str(s).unwrap_or_default() }

/// Validate a requested list: 1024-65535, at most 8, no duplicates, not the server's own game port and not used (as
/// game port or extra port) by any other Moss server.
pub fn validate(list: &[ExtraPort], own_port: i64, self_id: &str, all: &[Srv]) -> Result<(), String> {
    if list.len() > MAX_EXTRA { return Err(format!("at most {MAX_EXTRA} extra ports")); }
    for (i, p) in list.iter().enumerate() {
        if p.port < 1024 { return Err(format!("port {} is below 1024", p.port)); }
        if list[..i].contains(p) { return Err(format!("{}/{} is listed twice", p.port, p.proto.name())); }
        if p.proto == Proto::Tcp && i64::from(p.port) == own_port { return Err(format!("{} is already this server's game port", p.port)); }
        for o in all.iter().filter(|o| o.id != self_id) {
            if o.port == i64::from(p.port) && p.proto == Proto::Tcp { return Err(format!("{} is the game port of {}", p.port, o.name)); }
            if parse_stored(&o.extra_ports).contains(p) { return Err(format!("{}/{} is already used by {}", p.port, p.proto.name(), o.name)); }
        }
    }
    Ok(())
}

/// Does `port` (TCP) collide with another server's extra ports? Used when a game port changes.
pub fn clashes_with_extra(port: i64, self_id: &str, all: &[Srv]) -> bool {
    all.iter().filter(|o| o.id != self_id).any(|o| parse_stored(&o.extra_ports).iter().any(|e| e.proto == Proto::Tcp && i64::from(e.port) == port))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(port: u16, proto: Proto) -> ExtraPort { ExtraPort { port, proto } }
    #[test]
    fn rules() {
        assert!(validate(&[p(19132, Proto::Udp), p(24454, Proto::Udp), p(24454, Proto::Tcp)], 25565, "a", &[]).is_ok());
        assert!(validate(&[p(80, Proto::Tcp)], 25565, "a", &[]).is_err());
        assert!(validate(&[p(25565, Proto::Tcp)], 25565, "a", &[]).is_err());
        assert!(validate(&[p(25565, Proto::Udp)], 25565, "a", &[]).is_ok());
        assert!(validate(&[p(2000, Proto::Udp), p(2000, Proto::Udp)], 25565, "a", &[]).is_err());
        let nine: Vec<ExtraPort> = (0..9).map(|i| p(2000 + i, Proto::Udp)).collect();
        assert!(validate(&nine, 25565, "a", &[]).is_err());
        assert_eq!(parse_stored("garbage"), vec![]);
        assert_eq!(Network::parse("game-only"), Some(Network::GameOnly));
        assert_eq!(Network::parse("host"), None);
    }
}
