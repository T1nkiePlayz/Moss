//! Minecraft Server List Ping (status protocol) used for readiness and player counts.
use serde_json::Value;
use std::time::Duration;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, time::timeout};

fn varint(mut v: u32, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 { out.push(b); break; }
        out.push(b | 0x80);
    }
}

async fn read_varint(s: &mut TcpStream) -> anyhow::Result<u32> {
    let (mut v, mut shift) = (0u32, 0);
    loop {
        let b = s.read_u8().await?;
        v |= ((b & 0x7f) as u32) << shift;
        if b & 0x80 == 0 { return Ok(v); }
        shift += 7;
        if shift > 35 { anyhow::bail!("bad varint"); }
    }
}

pub struct Status { pub online: u32, pub max: u32, pub sample: Vec<String>, pub motd: String }

pub async fn ping(port: u16) -> anyhow::Result<Status> {
    timeout(Duration::from_millis(1500), async {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await?;
        let mut hs = vec![0x00];
        varint(767, &mut hs);
        varint(9, &mut hs);
        hs.extend_from_slice(b"localhost");
        hs.extend_from_slice(&port.to_be_bytes());
        varint(1, &mut hs);
        let mut pkt = Vec::new();
        varint(hs.len() as u32, &mut pkt);
        pkt.extend(hs);
        pkt.extend_from_slice(&[1, 0]); // status request
        s.write_all(&pkt).await?;
        let _len = read_varint(&mut s).await?;
        let id = read_varint(&mut s).await?;
        if id != 0 { anyhow::bail!("unexpected packet"); }
        let slen = read_varint(&mut s).await? as usize;
        if slen > 1 << 20 { anyhow::bail!("status too large"); }
        let mut buf = vec![0u8; slen];
        s.read_exact(&mut buf).await?;
        let v: Value = serde_json::from_slice(&buf)?;
        let motd = match &v["description"] {
            Value::String(t) => t.clone(),
            d => d["text"].as_str().unwrap_or("").to_string() + &d["extra"].as_array().map(|a| a.iter().filter_map(|e| e["text"].as_str()).collect::<String>()).unwrap_or_default(),
        };
        Ok(Status {
            online: v["players"]["online"].as_u64().unwrap_or(0) as u32,
            max: v["players"]["max"].as_u64().unwrap_or(0) as u32,
            sample: v["players"]["sample"].as_array().map(|a| a.iter().filter_map(|p| p["name"].as_str().map(String::from)).collect()).unwrap_or_default(),
            motd,
        })
    }).await?
}

/// Minimal status responder for the simulated server.
pub async fn serve_sim(port: u16) {
    let Ok(l) = tokio::net::TcpListener::bind(("0.0.0.0", port)).await else { eprintln!("sim: cannot bind {port}"); return };
    loop {
        let Ok((mut s, _)) = l.accept().await else { continue };
        tokio::spawn(async move {
            let _ = async {
                let _ = read_varint(&mut s).await?;
                let mut b = [0u8; 256];
                let _ = timeout(Duration::from_millis(500), s.read(&mut b)).await;
                let json = br#"{"version":{"name":"sim-1.0","protocol":767},"players":{"max":20,"online":2,"sample":[{"name":"Steve","id":"1"},{"name":"Alex","id":"2"}]},"description":{"text":"A simulated Moss server"}}"#;
                let mut body = vec![0u8];
                varint(json.len() as u32, &mut body);
                body.extend_from_slice(json);
                let mut out = Vec::new();
                varint(body.len() as u32, &mut out);
                out.extend(body);
                s.write_all(&out).await?;
                anyhow::Ok(())
            }.await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn ping_talks_to_the_simulated_server() {
        let port = 39517;
        tokio::spawn(serve_sim(port));
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let s = ping(port).await.unwrap();
        assert_eq!((s.online, s.max), (2, 20));
        assert_eq!(s.sample, vec!["Steve", "Alex"]);
    }
}
