//! Long-term metrics. The 5 s samples kept in memory (`st.stats`, plus a host ring here) are folded into hourly
//! buckets; completed hours are upserted into `metrics_hourly` / `host_hourly` (partial hours are flushed on
//! graceful shutdown and merge with later data), and rows older than 90 days are pruned. Queries return raw
//! samples for 1 h and hourly roll-ups for longer ranges, downsampled to at most 300 points.
use crate::state::{Sample, St};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{collections::{HashMap, VecDeque}, sync::{Mutex, OnceLock}, time::Duration};

pub const RETENTION_DAYS: i64 = 90;
pub const MAX_POINTS: usize = 300;
const RAW_STEP_S: i64 = 5;

/// Running aggregate of samples (one hour of one series).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Agg { pub n: u32, pub cpu_sum: f64, pub cpu_max: f64, pub mem_sum: f64, pub mem_max: u64, pub players_max: u32, pub disk_free: Option<u64> }

impl Agg {
    pub fn add(&mut self, cpu: f64, mem: u64, players: u32) {
        self.n += 1; self.cpu_sum += cpu; self.cpu_max = self.cpu_max.max(cpu);
        self.mem_sum += mem as f64; self.mem_max = self.mem_max.max(mem); self.players_max = self.players_max.max(players);
    }
    pub fn merge(&mut self, o: &Agg) {
        self.n += o.n; self.cpu_sum += o.cpu_sum; self.cpu_max = self.cpu_max.max(o.cpu_max);
        self.mem_sum += o.mem_sum; self.mem_max = self.mem_max.max(o.mem_max); self.players_max = self.players_max.max(o.players_max);
        if o.disk_free.is_some() { self.disk_free = o.disk_free; }
    }
    pub fn cpu_avg(&self) -> f64 { if self.n == 0 { 0.0 } else { self.cpu_sum / self.n as f64 } }
    pub fn mem_avg(&self) -> f64 { if self.n == 0 { 0.0 } else { self.mem_sum / self.n as f64 } }
}

/// One point of a series as served to clients.
#[derive(Clone, Debug, PartialEq)]
pub struct Pt { pub t: i64, pub agg: Agg }

impl Pt {
    fn json(&self, host: bool) -> Value {
        let a = &self.agg;
        let mut v = json!({"t": self.t, "cpu_avg": round1(a.cpu_avg()), "cpu_max": round1(a.cpu_max), "mem_avg": a.mem_avg().round(), "mem_max": a.mem_max});
        if host { v["disk_free"] = json!(a.disk_free.unwrap_or(0)); } else { v["players_max"] = json!(a.players_max); }
        v
    }
}
fn round1(x: f64) -> f64 { (x * 10.0).round() / 10.0 }

pub fn hour_of(ts_ms: i64) -> i64 { ts_ms.div_euclid(3_600_000) }

/// Merge consecutive points into groups of `k = ceil(len / max)` so at most `max` remain. Returns `(points, k)`.
pub fn downsample(pts: Vec<Pt>, max: usize) -> (Vec<Pt>, usize) {
    if pts.len() <= max { return (pts, 1); }
    let k = pts.len().div_ceil(max);
    let out = pts.chunks(k).map(|g| { let mut a = Agg::default(); for p in g { a.merge(&p.agg); } Pt { t: g[0].t, agg: a } }).collect();
    (out, k)
}

/// Combine rows that share an hour (database row + not yet flushed memory) and sort by time.
pub fn merge_hours(rows: Vec<(i64, Agg)>) -> Vec<(i64, Agg)> {
    let mut m: std::collections::BTreeMap<i64, Agg> = Default::default();
    for (h, a) in rows { m.entry(h).or_default().merge(&a); }
    m.into_iter().collect()
}

#[derive(Default)]
struct Hist {
    servers: HashMap<(String, i64), Agg>,
    host: HashMap<i64, Agg>,
    watermark: HashMap<String, i64>,
    host_ring: VecDeque<(i64, f64, u64, u64)>,
}
static HIST: OnceLock<Mutex<Hist>> = OnceLock::new();
fn hist() -> std::sync::MutexGuard<'static, Hist> { HIST.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner()) }

fn ingest_samples(h: &mut Hist, id: &str, samples: &[Sample]) {
    let mark = h.watermark.get(id).copied().unwrap_or(0);
    let mut newest = mark;
    for s in samples.iter().filter(|s| s.t > mark) {
        h.servers.entry((id.to_string(), hour_of(s.t))).or_default().add(s.cpu, s.mem, s.players.unwrap_or(0));
        newest = newest.max(s.t);
    }
    h.watermark.insert(id.to_string(), newest);
}

fn ingest(st: &St) {
    let snap: Vec<(String, Vec<Sample>)> = st.stats.lock().unwrap().iter().map(|(k, q)| (k.clone(), q.iter().copied().collect())).collect();
    let mut h = hist();
    for (id, s) in snap { ingest_samples(&mut h, &id, &s); }
}

fn host_tick(st: &St) {
    let cpu = st.host.lock().unwrap().cpu_pct;
    let (_, used) = crate::platform::host_memory();
    let (_, free) = crate::platform::disk(&st.cfg.data_dir);
    let t = chrono::Utc::now().timestamp_millis();
    let mut h = hist();
    h.host_ring.push_back((t, cpu, used, free));
    while h.host_ring.len() > 720 { h.host_ring.pop_front(); }
    let a = h.host.entry(hour_of(t)).or_default();
    a.add(cpu, used, 0);
    a.disk_free = Some(free);
}

const UPSERT_SERVER: &str = "INSERT INTO metrics_hourly(server_id,hour,n,cpu_avg,cpu_max,mem_avg,mem_max,players_max) VALUES (?,?,?,?,?,?,?,?)
  ON CONFLICT(server_id,hour) DO UPDATE SET cpu_avg=(cpu_avg*n+excluded.cpu_avg*excluded.n)/(n+excluded.n), mem_avg=(mem_avg*n+excluded.mem_avg*excluded.n)/(n+excluded.n),
  n=n+excluded.n, cpu_max=max(cpu_max,excluded.cpu_max), mem_max=max(mem_max,excluded.mem_max), players_max=max(players_max,excluded.players_max)";
const UPSERT_HOST: &str = "INSERT INTO host_hourly(hour,n,cpu_avg,cpu_max,mem_avg,mem_max,disk_free) VALUES (?,?,?,?,?,?,?)
  ON CONFLICT(hour) DO UPDATE SET cpu_avg=(cpu_avg*n+excluded.cpu_avg*excluded.n)/(n+excluded.n), mem_avg=(mem_avg*n+excluded.mem_avg*excluded.n)/(n+excluded.n),
  n=n+excluded.n, cpu_max=max(cpu_max,excluded.cpu_max), mem_max=max(mem_max,excluded.mem_max), disk_free=excluded.disk_free";

type ServerRows = Vec<((String, i64), Agg)>;
type HostRows = Vec<(i64, Agg)>;

fn write_rows(c: &Connection, s: &ServerRows, h: &HostRows) -> rusqlite::Result<()> {
    let tx = c.unchecked_transaction()?;
    for ((id, hour), a) in s { tx.execute(UPSERT_SERVER, params![id, hour, a.n, a.cpu_avg(), a.cpu_max, a.mem_avg(), a.mem_max as i64, a.players_max])?; }
    for (hour, a) in h { tx.execute(UPSERT_HOST, params![hour, a.n, a.cpu_avg(), a.cpu_max, a.mem_avg(), a.mem_max as i64, a.disk_free.unwrap_or(0) as i64])?; }
    tx.commit()
}

/// Write completed hours (or everything, on shutdown) to the database.
pub async fn flush(st: &St, everything: bool) {
    ingest(st);
    let cur = hour_of(chrono::Utc::now().timestamp_millis());
    let (s, h): (ServerRows, HostRows) = {
        let mut g = hist();
        let sk: Vec<_> = g.servers.keys().filter(|(_, hr)| everything || *hr < cur).cloned().collect();
        let hk: Vec<_> = g.host.keys().filter(|hr| everything || **hr < cur).copied().collect();
        (sk.into_iter().filter_map(|k| g.servers.remove(&k).map(|a| (k, a))).collect(), hk.into_iter().filter_map(|k| g.host.remove(&k).map(|a| (k, a))).collect())
    };
    if s.is_empty() && h.is_empty() { return; }
    let (s2, h2) = (s.clone(), h.clone());
    if let Err(e) = st.db.call(move |c| write_rows(c, &s2, &h2)).await {
        tracing::warn!("could not store hourly metrics: {e:#}");
        let mut g = hist(); // keep them for the next attempt
        for (k, a) in s { g.servers.entry(k).or_default().merge(&a); }
        for (k, a) in h { g.host.entry(k).or_default().merge(&a); }
    }
}

pub fn prune(c: &Connection) -> rusqlite::Result<()> {
    let oldest = hour_of(chrono::Utc::now().timestamp_millis()) - RETENTION_DAYS * 24;
    c.execute("DELETE FROM metrics_hourly WHERE hour < ?", [oldest])?;
    c.execute("DELETE FROM host_hourly WHERE hour < ?", [oldest])?;
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(RETENTION_DAYS)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    c.execute("DELETE FROM notifications WHERE time < ? OR id NOT IN (SELECT id FROM notifications ORDER BY time DESC LIMIT 1000)", [cutoff])?;
    Ok(())
}

pub fn spawn_background(st: &St) {
    let st = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut last_hour = hour_of(chrono::Utc::now().timestamp_millis());
        loop {
            tick.tick().await;
            host_tick(&st);
            ingest(&st);
            let hr = hour_of(chrono::Utc::now().timestamp_millis());
            if hr != last_hour { last_hour = hr; flush(&st, false).await; }
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Range { H1, H24, D7, D30 }
impl Range {
    pub fn parse(s: &str) -> Option<Range> { match s { "1h" => Some(Range::H1), "24h" => Some(Range::H24), "7d" => Some(Range::D7), "30d" => Some(Range::D30), _ => None } }
    fn hours(self) -> i64 { match self { Range::H1 => 1, Range::H24 => 24, Range::D7 => 168, Range::D30 => 720 } }
}

fn series_json(range: &str, pts: Vec<Pt>, base_step: i64, host: bool) -> Value {
    let (pts, k) = downsample(pts, MAX_POINTS);
    json!({"range": range, "step_s": base_step * k as i64, "samples": pts.iter().map(|p| p.json(host)).collect::<Vec<_>>()})
}

fn raw_pts(samples: impl Iterator<Item = (i64, f64, u64, u32, Option<u64>)>, since: i64) -> Vec<Pt> {
    samples.filter(|s| s.0 >= since).map(|(t, cpu, mem, pl, disk)| { let mut a = Agg::default(); a.add(cpu, mem, pl); a.disk_free = disk; Pt { t, agg: a } }).collect()
}

fn hourly_pts(rows: Vec<(i64, Agg)>) -> Vec<Pt> { merge_hours(rows).into_iter().map(|(h, agg)| Pt { t: h * 3_600_000, agg }).collect() }

pub async fn server_series(st: &St, id: &str, range_s: &str, range: Range) -> anyhow::Result<Value> {
    let now = chrono::Utc::now().timestamp_millis();
    if range == Range::H1 {
        let raw: Vec<Sample> = st.stats.lock().unwrap().get(id).map(|q| q.iter().copied().collect()).unwrap_or_default();
        return Ok(series_json(range_s, raw_pts(raw.into_iter().map(|s| (s.t, s.cpu, s.mem, s.players.unwrap_or(0), None)), now - 3_600_000), RAW_STEP_S, false));
    }
    ingest(st);
    let since = hour_of(now) - range.hours() + 1;
    let sid = id.to_string();
    let mut rows: Vec<(i64, Agg)> = st.db.call(move |c| {
        let mut q = c.prepare("SELECT hour,n,cpu_avg,cpu_max,mem_avg,mem_max,players_max FROM metrics_hourly WHERE server_id=? AND hour>=? ORDER BY hour")?;
        q.query_map(params![sid, since], |r| { let n: u32 = r.get(1)?; Ok((r.get::<_, i64>(0)?, Agg { n, cpu_sum: r.get::<_, f64>(2)? * n as f64, cpu_max: r.get(3)?, mem_sum: r.get::<_, f64>(4)? * n as f64, mem_max: r.get::<_, i64>(5)? as u64, players_max: r.get(6)?, disk_free: None })) })?.collect()
    }).await?;
    rows.extend(hist().servers.iter().filter(|((s, h), _)| s == id && *h >= since).map(|((_, h), a)| (*h, *a)));
    Ok(series_json(range_s, hourly_pts(rows), 3600, false))
}

pub async fn host_series(st: &St, range_s: &str, range: Range) -> anyhow::Result<Value> {
    let now = chrono::Utc::now().timestamp_millis();
    if range == Range::H1 {
        let raw: Vec<_> = hist().host_ring.iter().map(|&(t, cpu, mem, free)| (t, cpu, mem, 0u32, Some(free))).collect();
        return Ok(series_json(range_s, raw_pts(raw.into_iter(), now - 3_600_000), RAW_STEP_S, true));
    }
    let since = hour_of(now) - range.hours() + 1;
    let mut rows: Vec<(i64, Agg)> = st.db.call(move |c| {
        let mut q = c.prepare("SELECT hour,n,cpu_avg,cpu_max,mem_avg,mem_max,disk_free FROM host_hourly WHERE hour>=? ORDER BY hour")?;
        q.query_map([since], |r| { let n: u32 = r.get(1)?; Ok((r.get::<_, i64>(0)?, Agg { n, cpu_sum: r.get::<_, f64>(2)? * n as f64, cpu_max: r.get(3)?, mem_sum: r.get::<_, f64>(4)? * n as f64, mem_max: r.get::<_, i64>(5)? as u64, players_max: 0, disk_free: Some(r.get::<_, i64>(6)? as u64) })) })?.collect()
    }).await?;
    rows.extend(hist().host.iter().filter(|(h, _)| **h >= since).map(|(h, a)| (*h, *a)));
    Ok(series_json(range_s, hourly_pts(rows), 3600, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agg(vals: &[(f64, u64, u32)]) -> Agg { let mut a = Agg::default(); for &(c, m, p) in vals { a.add(c, m, p); } a }

    #[test]
    fn aggregate_averages_and_maxima() {
        let a = agg(&[(10.0, 100, 1), (30.0, 300, 4), (20.0, 200, 2)]);
        assert_eq!((a.n, a.cpu_avg(), a.cpu_max, a.mem_avg(), a.mem_max, a.players_max), (3, 20.0, 30.0, 200.0, 300, 4));
        assert_eq!(Agg::default().cpu_avg(), 0.0);
    }

    #[test]
    fn merging_is_weighted_by_sample_count() {
        let mut a = agg(&[(10.0, 100, 1)]);
        a.merge(&agg(&[(40.0, 400, 3), (40.0, 400, 2), (40.0, 400, 1)]));
        assert_eq!((a.n, a.cpu_avg(), a.cpu_max, a.mem_max, a.players_max), (4, 32.5, 40.0, 400, 3));
    }

    #[test]
    fn hours_split_on_the_hour_boundary() {
        assert_eq!(hour_of(0), 0);
        assert_eq!(hour_of(3_599_999), 0);
        assert_eq!(hour_of(3_600_000), 1);
        assert_eq!(hour_of(-1), -1);
    }

    #[test]
    fn downsampling_bounds_the_point_count_and_keeps_extremes() {
        let pts: Vec<Pt> = (0..720).map(|i| Pt { t: i * 5000, agg: agg(&[(if i == 100 { 99.0 } else { 1.0 }, 10, 0)]) }).collect();
        let (out, k) = downsample(pts, 300);
        assert_eq!(k, 3);
        assert_eq!(out.len(), 240);
        assert_eq!(out.iter().map(|p| p.agg.cpu_max).fold(0.0, f64::max), 99.0);
        assert_eq!(out[0].t, 0);
        let small: Vec<Pt> = (0..10).map(|i| Pt { t: i, agg: agg(&[(1.0, 1, 0)]) }).collect();
        assert_eq!(downsample(small, 300).1, 1);
        // 30 days of hourly rows
        let month: Vec<Pt> = (0..720).map(|i| Pt { t: i * 3_600_000, agg: agg(&[(1.0, 1, 0)]) }).collect();
        assert!(downsample(month, MAX_POINTS).0.len() <= MAX_POINTS);
    }

    #[test]
    fn rows_for_the_same_hour_are_combined() {
        let m = merge_hours(vec![(5, agg(&[(10.0, 1, 1)])), (4, agg(&[(1.0, 1, 1)])), (5, agg(&[(30.0, 1, 5)]))]);
        assert_eq!(m.iter().map(|x| x.0).collect::<Vec<_>>(), vec![4, 5]);
        assert_eq!((m[1].1.n, m[1].1.cpu_avg(), m[1].1.players_max), (2, 20.0, 5));
    }

    #[test]
    fn ingest_only_counts_each_sample_once() {
        let mut h = Hist::default();
        let s = |t, cpu| Sample { t, cpu, mem: 1000, players: Some(2) };
        ingest_samples(&mut h, "a", &[s(1000, 10.0), s(2000, 20.0)]);
        ingest_samples(&mut h, "a", &[s(1000, 10.0), s(2000, 20.0), s(3_600_500, 50.0)]);
        assert_eq!(h.servers[&("a".to_string(), 0)].n, 2);
        assert_eq!(h.servers[&("a".to_string(), 1)].n, 1);
        assert_eq!(h.servers[&("a".to_string(), 0)].cpu_avg(), 15.0);
    }

    #[test]
    fn ranges_parse() {
        assert_eq!(Range::parse("24h"), Some(Range::H24));
        assert_eq!(Range::parse("90d"), None);
    }

    #[tokio::test]
    async fn upsert_merges_a_partial_hour_with_later_data_and_prune_keeps_recent() {
        let p = std::env::temp_dir().join(format!("moss-hist-{}.sqlite", uuid::Uuid::new_v4()));
        let d = crate::db::Db::open(&p).unwrap();
        let first = vec![(("s".to_string(), 7i64), agg(&[(10.0, 100, 1)]))];
        let second = vec![(("s".to_string(), 7i64), agg(&[(30.0, 300, 3), (30.0, 300, 3), (30.0, 300, 3)]))];
        let (f2, s2) = (first.clone(), second.clone());
        let row = d.call(move |c| {
            write_rows(c, &f2, &vec![])?;
            write_rows(c, &s2, &vec![])?;
            c.query_row("SELECT n,cpu_avg,cpu_max,mem_max,players_max FROM metrics_hourly WHERE server_id='s' AND hour=7", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?)))
        }).await.unwrap();
        assert_eq!(row, (4, 25.0, 30.0, 300, 3));
        let left = d.call(|c| { prune(c)?; c.query_row("SELECT COUNT(*) FROM metrics_hourly", [], |r| r.get::<_, i64>(0)) }).await.unwrap();
        assert_eq!(left, 0, "hour 7 is far older than 90 days");
        let _ = std::fs::remove_file(p);
    }
}
