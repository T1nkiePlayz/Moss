//! Loopback health probing with plain std TCP: the controller answers `GET /healthz` with `ok`.
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// A port nobody is listening on right now (the controller binds it a moment later; a rare race is retried by the caller).
pub fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?.port())
}

/// True when something on 127.0.0.1:port answers `/healthz` with 200 and the body `ok`.
pub fn healthy(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    let _ = s.set_write_timeout(Some(Duration::from_millis(800)));
    if s.write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").is_err() { return false; }
    let mut buf = Vec::new();
    let _ = s.take(4096).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    text.starts_with("HTTP/1.1 200") && text.trim_end().ends_with("ok")
}

/// Poll until the controller is healthy, `alive` turns false (it exited) or `total` passes.
pub fn wait_ready(port: u16, total: Duration, mut alive: impl FnMut() -> bool) -> Result<(), String> {
    let end = Instant::now() + total;
    let mut pause = Duration::from_millis(25); // quick first polls, backing off to 150 ms
    while Instant::now() < end {
        if healthy(port) { return Ok(()); }
        if !alive() { return Err("The Moss controller stopped while starting. See moss.log in the Moss data folder.".into()); }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(150));
    }
    Err("The Moss controller did not become ready in time. See moss.log in the Moss data folder.".into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    /// A one-thread stand-in for the controller: answers every request with `status` and `body`.
    pub(crate) fn stub(status: &str, body: &str) -> u16 {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        let resp = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { break };
                let mut b = [0u8; 512];
                let _ = s.read(&mut b);
                let _ = s.write_all(resp.as_bytes());
            }
        });
        port
    }
    #[test]
    fn healthy_only_for_200_ok() {
        assert!(healthy(stub("200 OK", "ok")));
        assert!(!healthy(stub("200 OK", "something else")));
        assert!(!healthy(stub("503 Service Unavailable", "ok")));
        assert!(!healthy(free_port().unwrap()));
    }
    #[test]
    fn wait_ready_stops_when_the_process_is_gone() {
        let p = free_port().unwrap();
        assert!(wait_ready(p, Duration::from_secs(5), || false).unwrap_err().contains("stopped"));
        assert!(wait_ready(p, Duration::from_millis(300), || true).unwrap_err().contains("in time"));
        assert!(wait_ready(stub("200 OK", "ok"), Duration::from_secs(2), || true).is_ok());
    }
}
