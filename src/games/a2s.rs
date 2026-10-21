//! Steam server query (A2S_INFO and A2S_PLAYER) over UDP, including the S2C_CHALLENGE handshake that servers have
//! required since late 2020. Used for readiness and player counts of Source, Valheim, Rust and similar servers.
use anyhow::{Result, anyhow, bail};
use std::{net::SocketAddr, time::Duration};
use tokio::{net::UdpSocket, time::timeout};

const HEADER: [u8; 4] = [0xFF; 4];
const T_INFO: u8 = 0x54;
const T_PLAYER: u8 = 0x55;
const R_INFO: u8 = 0x49;
const R_CHALLENGE: u8 = 0x41;
const R_PLAYER: u8 = 0x44;

#[derive(Clone, Debug, PartialEq)]
pub struct Info { pub name: String, pub map: String, pub game: String, pub players: u32, pub max: u32, pub bots: u32 }

/// Request packet: `FF FF FF FF <type> <payload> [challenge]`.
pub fn request(kind: u8, challenge: Option<[u8; 4]>) -> Vec<u8> {
    let mut v = HEADER.to_vec();
    v.push(kind);
    if kind == T_INFO { v.extend_from_slice(b"Source Engine Query\0"); }
    if kind == T_PLAYER && challenge.is_none() { v.extend_from_slice(&HEADER); } // "give me a challenge"
    if let Some(c) = challenge { v.extend_from_slice(&c); }
    v
}

/// Splits a response into (type byte, body). Multi-packet responses are not used for these small queries.
pub fn split(buf: &[u8]) -> Result<(u8, &[u8])> {
    if buf.len() < 5 { bail!("short A2S packet"); }
    if buf[..4] == [0xFF, 0xFF, 0xFF, 0xFE] { bail!("split A2S responses are not supported"); }
    if buf[..4] != HEADER { bail!("not an A2S packet"); }
    Ok((buf[4], &buf[5..]))
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn u8(&mut self) -> Result<u8> { let (&b, rest) = self.0.split_first().ok_or_else(|| anyhow!("truncated A2S packet"))?; self.0 = rest; Ok(b) }
    fn skip(&mut self, n: usize) -> Result<()> { if self.0.len() < n { bail!("truncated A2S packet"); } self.0 = &self.0[n..]; Ok(()) }
    fn cstr(&mut self) -> Result<String> {
        let end = self.0.iter().position(|&b| b == 0).ok_or_else(|| anyhow!("unterminated string in A2S packet"))?;
        let s = String::from_utf8_lossy(&self.0[..end]).into_owned();
        self.0 = &self.0[end + 1..];
        Ok(s)
    }
}

/// Parses the body of an A2S_INFO reply (the modern Source layout; GoldSrc replies are rejected).
pub fn parse_info(body: &[u8]) -> Result<Info> {
    let mut r = Reader(body);
    r.skip(1)?; // protocol
    let (name, map) = (r.cstr()?, r.cstr()?);
    let _folder = r.cstr()?;
    let game = r.cstr()?;
    r.skip(2)?; // app id
    let (players, max, bots) = (r.u8()? as u32, r.u8()? as u32, r.u8()? as u32);
    Ok(Info { name, map, game, players: players.saturating_sub(bots), max, bots })
}

/// Parses the body of an A2S_PLAYER reply into player names (unnamed connecting slots are dropped).
pub fn parse_players(body: &[u8]) -> Result<Vec<String>> {
    let mut r = Reader(body);
    let n = r.u8()?;
    let mut out = vec![];
    for _ in 0..n {
        r.skip(1)?; // index
        let name = r.cstr()?;
        r.skip(8)?; // score + duration
        if !name.is_empty() { out.push(name); }
    }
    Ok(out)
}

async fn exchange(sock: &UdpSocket, kind: u8, want: u8) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; 1400];
    sock.send(&request(kind, None)).await?;
    let mut challenge = None;
    for _ in 0..2 {
        let n = sock.recv(&mut buf).await?;
        let (k, body) = split(&buf[..n])?;
        if k == want { return Ok(body.to_vec()); }
        if k != R_CHALLENGE || body.len() < 4 || challenge.is_some() { bail!("unexpected A2S reply type {k:#x}"); }
        challenge = Some([body[0], body[1], body[2], body[3]]);
        sock.send(&request(kind, challenge)).await?;
    }
    Err(anyhow!("A2S challenge was not accepted"))
}

async fn connect(addr: SocketAddr) -> Result<UdpSocket> {
    let sock = UdpSocket::bind(if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).await?;
    sock.connect(addr).await?;
    Ok(sock)
}

/// A2S_INFO against `addr` with an overall 1.5 s deadline.
pub async fn info(addr: SocketAddr) -> Result<Info> {
    timeout(Duration::from_millis(1500), async {
        let sock = connect(addr).await?;
        parse_info(&exchange(&sock, T_INFO, R_INFO).await?)
    }).await.map_err(|_| anyhow!("A2S query timed out"))?
}

/// A2S_PLAYER names (best effort, 1.5 s deadline).
pub async fn players(addr: SocketAddr) -> Result<Vec<String>> {
    timeout(Duration::from_millis(1500), async {
        let sock = connect(addr).await?;
        parse_players(&exchange(&sock, T_PLAYER, R_PLAYER).await?)
    }).await.map_err(|_| anyhow!("A2S query timed out"))?
}

/// Info plus (when cheap and answered) player names, as the monitor wants them.
pub async fn query(addr: SocketAddr) -> Result<(Info, Vec<String>)> {
    let i = info(addr).await?;
    let names = if i.players > 0 { players(addr).await.unwrap_or_default() } else { vec![] };
    Ok((i, names))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_packet(players: u8, max: u8, bots: u8) -> Vec<u8> {
        let mut v = vec![0xFF, 0xFF, 0xFF, 0xFF, R_INFO, 17];
        for s in ["My Server", "gm_construct", "garrysmod", "Sandbox"] { v.extend_from_slice(s.as_bytes()); v.push(0); }
        v.extend_from_slice(&[0x10, 0x0F]); // app id 3856 (little endian)
        v.extend_from_slice(&[players, max, bots, b'd', b'l', 0, 1]);
        v.extend_from_slice(b"1.0.0.0\0");
        v
    }

    #[test]
    fn request_packets() {
        let r = request(T_INFO, None);
        assert_eq!(&r[..5], &[0xFF, 0xFF, 0xFF, 0xFF, 0x54]);
        assert!(r.ends_with(b"Source Engine Query\0"));
        assert_eq!(request(T_INFO, Some([1, 2, 3, 4])).split_off(r.len()), vec![1, 2, 3, 4]);
        assert_eq!(request(T_PLAYER, None), vec![0xFF, 0xFF, 0xFF, 0xFF, 0x55, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(request(T_PLAYER, Some([9, 8, 7, 6])), vec![0xFF, 0xFF, 0xFF, 0xFF, 0x55, 9, 8, 7, 6]);
    }

    #[test]
    fn parses_info_fixture() {
        let p = info_packet(5, 16, 1);
        let (k, body) = split(&p).unwrap();
        assert_eq!(k, R_INFO);
        let i = parse_info(body).unwrap();
        assert_eq!((i.name.as_str(), i.map.as_str(), i.game.as_str()), ("My Server", "gm_construct", "Sandbox"));
        assert_eq!((i.players, i.max, i.bots), (4, 16, 1)); // bots are not players
    }

    #[test]
    fn recognises_challenge() {
        let p = [0xFF, 0xFF, 0xFF, 0xFF, 0x41, 0xDE, 0xAD, 0xBE, 0xEF];
        let (k, body) = split(&p).unwrap();
        assert_eq!((k, body), (R_CHALLENGE, &[0xDE, 0xAD, 0xBE, 0xEF][..]));
    }

    #[test]
    fn parses_player_fixture() {
        let mut body = vec![2u8];
        for (i, n) in ["Alice", "Bob"].iter().enumerate() {
            body.push(i as u8);
            body.extend_from_slice(n.as_bytes());
            body.push(0);
            body.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0x80, 0x3F]);
        }
        assert_eq!(parse_players(&body).unwrap(), vec!["Alice".to_string(), "Bob".to_string()]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(split(b"nope").is_err());
        assert!(split(&[1, 2, 3, 4, 5]).is_err());
        assert!(split(&[0xFF, 0xFF, 0xFF, 0xFE, 0, 0]).is_err());
        assert!(parse_info(&[17, b'a']).is_err());
        assert!(parse_players(&[3, 0, b'x', 0]).is_err());
    }

    #[tokio::test]
    async fn full_handshake_against_a_local_fake_server() {
        let srv = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = srv.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 256];
            let (_, peer) = srv.recv_from(&mut buf).await.unwrap();
            srv.send_to(&[0xFF, 0xFF, 0xFF, 0xFF, 0x41, 1, 2, 3, 4], peer).await.unwrap();
            let (n, peer) = srv.recv_from(&mut buf).await.unwrap();
            assert!(buf[..n].ends_with(&[1, 2, 3, 4]));
            srv.send_to(&info_packet(3, 10, 0), peer).await.unwrap();
        });
        let i = info(addr).await.unwrap();
        assert_eq!((i.players, i.max), (3, 10));
    }
}
