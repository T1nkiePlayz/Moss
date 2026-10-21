//! Source RCON client (TCP, little-endian packets; type 3 auth, 2 exec, 0 response). Used for Source, Rust, Factorio,
//! Palworld and Zomboid servers on `127.0.0.1`. Every connect and read has a timeout; a multi-packet response is
//! ended by the echo of an empty probe packet (or, for servers that ignore the probe, a short idle gap).
use std::{net::SocketAddr, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, time::timeout};

const AUTH: i32 = 3;
const EXEC: i32 = 2;
const RESPONSE: i32 = 0;
const MAX_PACKET: usize = 64 * 1024;
const MAX_TOTAL: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum RconError { Connect(String), Timeout, AuthFailed, Protocol(String) }
impl std::fmt::Display for RconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RconError::Connect(e) => write!(f, "cannot connect to the server's RCON port: {e}"),
            RconError::Timeout => write!(f, "the server's RCON did not answer in time"),
            RconError::AuthFailed => write!(f, "RCON rejected the password (check rcon_password in moss-game.cfg and restart)"),
            RconError::Protocol(e) => write!(f, "RCON protocol error: {e}"),
        }
    }
}
impl std::error::Error for RconError {}

/// Timeouts, overridable so tests do not wait seconds.
#[derive(Clone, Copy)]
pub struct Timeouts { pub connect: Duration, pub read: Duration, pub idle: Duration }
impl Default for Timeouts { fn default() -> Self { Timeouts { connect: Duration::from_secs(3), read: Duration::from_secs(5), idle: Duration::from_millis(400) } } }

fn packet(id: i32, ty: i32, body: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(14 + body.len());
    p.extend_from_slice(&((10 + body.len()) as i32).to_le_bytes());
    p.extend_from_slice(&id.to_le_bytes());
    p.extend_from_slice(&ty.to_le_bytes());
    p.extend_from_slice(body.as_bytes());
    p.extend_from_slice(&[0, 0]);
    p
}

async fn read_packet(s: &mut TcpStream, wait: Duration) -> Result<(i32, i32, String), RconError> {
    let io = |e: std::io::Error| if e.kind() == std::io::ErrorKind::UnexpectedEof { RconError::Protocol("connection closed".into()) } else { RconError::Protocol(e.to_string()) };
    let mut len = [0u8; 4];
    timeout(wait, s.read_exact(&mut len)).await.map_err(|_| RconError::Timeout)?.map_err(io)?;
    let len = i32::from_le_bytes(len);
    if !(10..=MAX_PACKET as i32).contains(&len) { return Err(RconError::Protocol(format!("bad packet length {len}"))); }
    let mut buf = vec![0u8; len as usize];
    timeout(wait, s.read_exact(&mut buf)).await.map_err(|_| RconError::Timeout)?.map_err(io)?;
    let (id, ty) = (i32::from_le_bytes(buf[0..4].try_into().unwrap()), i32::from_le_bytes(buf[4..8].try_into().unwrap()));
    let body = &buf[8..buf.len() - 2];
    let body = body.split(|&b| b == 0).next().unwrap_or(&[]);
    Ok((id, ty, String::from_utf8_lossy(body).into_owned()))
}

/// Connects, authenticates and runs one command, returning its (possibly multi-packet) output.
pub async fn exec(addr: SocketAddr, password: &str, cmd: &str) -> Result<String, RconError> { exec_with(addr, password, cmd, Timeouts::default()).await }

/// The whole exchange is bounded too: a peer that keeps sending packets with other ids never resets a read timeout forever.
pub async fn exec_with(addr: SocketAddr, password: &str, cmd: &str, t: Timeouts) -> Result<String, RconError> {
    timeout(t.connect + t.read * 2, exchange(addr, password, cmd, t)).await.map_err(|_| RconError::Timeout)?
}

async fn exchange(addr: SocketAddr, password: &str, cmd: &str, t: Timeouts) -> Result<String, RconError> {
    let mut s = timeout(t.connect, TcpStream::connect(addr)).await.map_err(|_| RconError::Timeout)?.map_err(|e| RconError::Connect(e.to_string()))?;
    let w = |e: std::io::Error| RconError::Protocol(e.to_string());
    timeout(t.read, s.write_all(&packet(1, AUTH, password))).await.map_err(|_| RconError::Timeout)?.map_err(w)?;
    loop { // an empty RESPONSE_VALUE may precede the AUTH_RESPONSE
        let (id, ty, _) = read_packet(&mut s, t.read).await?;
        if ty == EXEC { if id == -1 { return Err(RconError::AuthFailed); } break; }
    }
    timeout(t.read, s.write_all(&[packet(2, EXEC, cmd), packet(3, RESPONSE, "")].concat())).await.map_err(|_| RconError::Timeout)?.map_err(w)?;
    let (mut out, mut got) = (String::new(), false);
    loop {
        let (id, ty, body) = match read_packet(&mut s, if got { t.idle } else { t.read }).await {
            Err(RconError::Timeout) if got => break, // a server that ignores the probe: the gap ends the response
            r => r?,
        };
        if id == 3 { break; }
        if ty == RESPONSE && id == 2 { got = true; out.push_str(&body); if out.len() > MAX_TOTAL { return Err(RconError::Protocol("response too large".into())); } }
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn read(s: &mut TcpStream) -> (i32, i32, String) {
        let mut l = [0u8; 4];
        s.read_exact(&mut l).await.unwrap();
        let mut b = vec![0u8; i32::from_le_bytes(l) as usize];
        s.read_exact(&mut b).await.unwrap();
        (i32::from_le_bytes(b[0..4].try_into().unwrap()), i32::from_le_bytes(b[4..8].try_into().unwrap()), String::from_utf8_lossy(&b[8..b.len() - 2]).into_owned())
    }

    /// In-process fake RCON server. `replies(cmd)` is the list of response bodies; `echo` mirrors the probe.
    pub async fn fake(password: &'static str, replies: fn(&str) -> Vec<String>, echo: bool) -> SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                tokio::spawn(async move {
                    let (id, ty, pw) = read(&mut s).await;
                    assert_eq!(ty, AUTH);
                    s.write_all(&packet(id, RESPONSE, "")).await.unwrap();
                    if pw != password { s.write_all(&packet(-1, EXEC, "")).await.unwrap(); return; }
                    s.write_all(&packet(id, EXEC, "")).await.unwrap();
                    let (cid, cty, cmd) = read(&mut s).await;
                    assert_eq!(cty, EXEC);
                    let (pid, pty, _) = read(&mut s).await;
                    assert_eq!(pty, RESPONSE);
                    for body in replies(&cmd) { s.write_all(&packet(cid, RESPONSE, &body)).await.unwrap(); }
                    if echo { s.write_all(&packet(pid, RESPONSE, "")).await.unwrap(); s.write_all(&packet(pid, RESPONSE, "\u{1}")).await.unwrap(); }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                });
            }
        });
        addr
    }

    const FAST: Timeouts = Timeouts { connect: Duration::from_millis(500), read: Duration::from_millis(500), idle: Duration::from_millis(100) };

    #[tokio::test]
    async fn auth_and_single_response() {
        let a = fake("pw", |c| vec![format!("ran {c}")], true).await;
        assert_eq!(exec_with(a, "pw", "status", FAST).await.unwrap(), "ran status");
    }

    #[tokio::test]
    async fn wrong_password_is_refused() {
        let a = fake("pw", |_| vec![], true).await;
        assert_eq!(exec_with(a, "nope", "status", FAST).await, Err(RconError::AuthFailed));
    }

    #[tokio::test]
    async fn multi_packet_response_is_joined_until_the_probe_echo() {
        let a = fake("pw", |_| vec!["a".repeat(4096), "b".repeat(4096), "tail".into()], true).await;
        let out = exec_with(a, "pw", "big", FAST).await.unwrap();
        assert_eq!(out.len(), 4096 * 2 + 4);
        assert!(out.starts_with('a') && out.ends_with("btail"));
    }

    #[tokio::test]
    async fn servers_that_ignore_the_probe_end_on_an_idle_gap() {
        let a = fake("pw", |_| vec!["one".into(), "two".into()], false).await;
        assert_eq!(exec_with(a, "pw", "x", FAST).await.unwrap(), "onetwo");
    }

    #[tokio::test]
    async fn silent_servers_time_out() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let a = l.local_addr().unwrap();
        tokio::spawn(async move { let (_s, _) = l.accept().await.unwrap(); tokio::time::sleep(Duration::from_secs(3)).await; });
        assert_eq!(exec_with(a, "pw", "status", FAST).await, Err(RconError::Timeout));
    }

    #[tokio::test]
    async fn refused_connections_are_reported() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let a = l.local_addr().unwrap();
        drop(l);
        assert!(matches!(exec_with(a, "pw", "x", FAST).await, Err(RconError::Connect(_))));
    }
}
