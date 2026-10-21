//! Readiness and player tracking for game servers, driven once a second from the supervisor. Log-based games get a
//! per-run tracker that reads new console lines (join/leave sets are rebuilt from the start of the log after a
//! controller restart); A2S games are probed over UDP.
use super::{Event, Game, PlayerSource, a2s, classify};
use crate::{lifecycle, server::{self, Srv}, state::{Players, St}};
use std::{
    collections::{BTreeSet, HashMap},
    io::{Read, Seek, SeekFrom},
    net::{Ipv4Addr, SocketAddr},
    sync::{LazyLock, Mutex},
};

const CHUNK: u64 = 256 * 1024;
const MAX_LINE: usize = 64 * 1024;

/// Console-line state for one run of one server.
pub struct Tracker { pid: i64, offset: u64, partial: Vec<u8>, pub names: BTreeSet<String>, pub ready: bool }

impl Tracker {
    pub fn new(pid: i64) -> Tracker { Tracker { pid, offset: 0, partial: vec![], names: BTreeSet::new(), ready: false } }

    /// Feeds raw log bytes; returns true when the set of online players changed.
    pub fn feed(&mut self, g: &Game, chunk: &[u8]) -> bool {
        self.partial.extend_from_slice(chunk);
        let mut changed = false;
        while let Some(i) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=i).collect();
            let line = String::from_utf8_lossy(&line);
            match classify(g, line.trim_end_matches(['\n', '\r'])) {
                Some(Event::Ready) => self.ready = true,
                Some(Event::Join(n)) => changed |= self.names.insert(n),
                Some(Event::Leave(n)) => changed |= self.names.remove(&n),
                None => {}
            }
        }
        if self.partial.len() > MAX_LINE { self.partial.clear(); }
        changed
    }
}

static TRACKERS: LazyLock<Mutex<HashMap<String, Tracker>>> = LazyLock::new(Default::default);

/// Drop a server's tracking state (it is not running).
pub fn forget(id: &str) { TRACKERS.lock().unwrap().remove(id); }

#[derive(Default)]
pub struct Update { pub ready: bool, pub names: Option<Vec<String>> }

/// Read new console lines for a running server. `names` is Some only when the player set changed (or is new).
pub async fn poll(st: &St, s: &Srv, g: &'static Game) -> Update {
    let Some(pid) = s.pid else { return Update::default() };
    let mut t = TRACKERS.lock().unwrap().remove(&s.id).filter(|t| t.pid == pid).unwrap_or_else(|| Tracker::new(pid));
    let (log, off) = (s.moss_dir(st).join("console.log"), t.offset);
    let read = tokio::task::spawn_blocking(move || {
        let mut f = std::fs::File::open(log).ok()?;
        let len = f.metadata().ok()?.len();
        let restart = len < off; // the log was replaced by a new run
        let from = if restart { 0 } else { off };
        f.seek(SeekFrom::Start(from)).ok()?;
        let mut buf = vec![];
        f.take(CHUNK).read_to_end(&mut buf).ok()?;
        Some((restart, from + buf.len() as u64, buf))
    }).await.ok().flatten();
    let mut changed = false;
    if let Some((restart, new_off, buf)) = read {
        if restart { t = Tracker::new(pid); changed = true; }
        t.offset = new_off;
        changed |= t.feed(g, &buf);
    }
    let up = Update { ready: t.ready, names: changed.then(|| t.names.iter().cloned().collect()) };
    TRACKERS.lock().unwrap().insert(s.id.clone(), t);
    up
}

/// One supervisor pass for a running game server: log-derived readiness and players, A2S readiness, fallback timer.
pub async fn tick(st: &St, s: &Srv, g: &'static Game, n: u64) {
    let (ready, secs, have_players) = {
        let live = st.live.lock().unwrap();
        let l = live.get(&s.id);
        (l.is_some_and(|l| l.ready), l.and_then(|l| l.started).map_or(0, |t| t.elapsed().as_secs()), l.is_some_and(|l| l.players.is_some()))
    };
    let up = if g.reads_log() { poll(st, s, g).await } else { Update::default() };
    if g.players == PlayerSource::Log && (up.names.is_some() || !have_players) {
        let names = up.names.unwrap_or_default();
        let (online, max) = (names.len() as u32, g.max_players);
        st.live.lock().unwrap().entry(s.id.clone()).or_default().players = Some(Players { online, max, sample: names });
        st.emit("server", server::view(s, st));
    }
    if ready { return; }
    let a2s_ok = match g.a2s_port(s.port) {
        Some(p) if n.is_multiple_of(3) && secs >= 4 => a2s::info(SocketAddr::from((Ipv4Addr::LOCALHOST, p))).await.is_ok(),
        _ => false,
    };
    if up.ready || a2s_ok || secs >= g.fallback_ready_s {
        lifecycle::mark_ready(st, &s.id);
        server::note_state(st, s);
    }
}

/// A2S player counts for every ready A2S game in `servers`, queried together.
pub async fn a2s_players(servers: &[&Srv]) -> HashMap<String, Players> {
    let targets: Vec<(&Srv, SocketAddr)> = servers.iter().filter_map(|s| Some((*s, SocketAddr::from((Ipv4Addr::LOCALHOST, s.game()?.a2s_port(s.port)?))))).collect();
    let res = futures_util::future::join_all(targets.iter().map(|(_, a)| a2s::query(*a))).await;
    targets.into_iter().zip(res).filter_map(|((s, _), r)| r.ok().map(|(i, names)| (s.id.clone(), Players { online: i.players, max: i.max, sample: names }))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::get;

    #[test]
    fn tracks_joins_leaves_and_readiness_across_chunks() {
        let g = get("bedrock").unwrap();
        let mut t = Tracker::new(1);
        assert!(!t.feed(g, b"[INFO] Server started.\n[INFO] Player connected: Al"));
        assert!(t.ready && t.names.is_empty());
        assert!(t.feed(g, b"ice, xuid: 1\r\n[INFO] Player connected: Bob, xuid: 2\n"));
        assert_eq!(t.names.iter().cloned().collect::<Vec<_>>(), vec!["Alice", "Bob"]);
        assert!(t.feed(g, b"[INFO] Player disconnected: Alice, xuid: 1\n"));
        assert!(!t.feed(g, b"[INFO] Player disconnected: Nobody, xuid: 9\n"));
        assert_eq!(t.names.len(), 1);
    }

    #[test]
    fn long_lines_do_not_grow_the_buffer() {
        let g = get("terraria").unwrap();
        let mut t = Tracker::new(1);
        t.feed(g, &vec![b'x'; MAX_LINE + 10]);
        assert!(t.partial.is_empty());
    }
}
