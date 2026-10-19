//! Durable operation engine: every long-running job is an Operation with a stable ID, persisted transitions,
//! bounded logs, cooperative cancellation and an SSE feed. Interrupted jobs are detected on startup.
use crate::{db::now, state::St};
use serde::Serialize;
use std::{collections::VecDeque, future::Future, time::Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Debug)]
pub struct Operation {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub server_id: Option<String>,
    pub state: String,
    pub stage: String,
    pub progress: Option<f64>,
    pub log: Vec<String>,
    pub error: Option<String>,
    pub cancellable: bool,
    pub created_at: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
}

#[derive(Default)]
pub struct Registry {
    pub items: VecDeque<Operation>,
    pub tokens: std::collections::HashMap<String, CancellationToken>,
    pub last_emit: std::collections::HashMap<String, Instant>,
}

#[derive(Clone)]
pub struct OpCtx {
    pub st: St,
    pub id: String,
    pub token: CancellationToken,
}

impl OpCtx {
    fn update(&self, f: impl FnOnce(&mut Operation), persist: bool, force: bool) {
        let snapshot = {
            let mut r = self.st.ops.lock().unwrap();
            let Some(op) = r.items.iter_mut().find(|o| o.id == self.id) else { return };
            f(op);
            let snap = op.clone();
            let due = force || r.last_emit.get(&self.id).is_none_or(|t| t.elapsed().as_millis() > 200);
            if due {
                r.last_emit.insert(self.id.clone(), Instant::now());
                Some(snap)
            } else {
                None
            }
        };
        if let Some(op) = snapshot {
            self.st.emit("op", serde_json::to_value(&op).unwrap());
            if persist {
                persist_op(&self.st, &op);
            }
        }
    }
    pub fn stage(&self, s: &str) {
        let s = s.to_string();
        self.update(|o| { o.stage = s.clone(); push_log(o, &s); }, true, true);
    }
    pub fn progress(&self, p: Option<f64>) { self.update(|o| o.progress = p, false, false); }
    pub fn log(&self, l: impl Into<String>) { let l = l.into(); self.update(|o| push_log(o, &l), false, false); }
    pub fn cancelled(&self) -> bool { self.token.is_cancelled() }
    pub fn check(&self) -> anyhow::Result<()> {
        if self.cancelled() { anyhow::bail!("cancelled") } else { Ok(()) }
    }
}

fn push_log(o: &mut Operation, l: &str) {
    o.log.push(l.to_string());
    if o.log.len() > 50 { o.log.remove(0); }
}

static WRITER: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<Operation>> = std::sync::OnceLock::new();

/// All persistence goes through one ordered writer so a late progress write can never overwrite a terminal state.
fn persist_op(st: &St, op: &Operation) {
    let tx = WRITER.get_or_init(|| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Operation>();
        let st = st.clone();
        tokio::spawn(async move {
            while let Some(op) = rx.recv().await {
                let _ = st.db.call(move |c| c.execute(
                    "INSERT INTO operations(id,kind,title,server_id,state,stage,progress,log,error,cancellable,created_at,started_at,ended_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)
                     ON CONFLICT(id) DO UPDATE SET state=excluded.state,stage=excluded.stage,progress=excluded.progress,log=excluded.log,error=excluded.error,started_at=excluded.started_at,ended_at=excluded.ended_at",
                    rusqlite::params![op.id, op.kind, op.title, op.server_id, op.state, op.stage, op.progress, serde_json::to_string(&op.log).unwrap(), op.error, op.cancellable, op.created_at, op.started_at, op.ended_at],
                )).await;
            }
        });
        tx
    });
    let _ = tx.send(op.clone());
}

pub fn spawn_op<F, Fut>(st: &St, kind: &str, title: impl Into<String>, server_id: Option<&str>, cancellable: bool, f: F) -> Operation
where
    F: FnOnce(OpCtx) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let op = Operation {
        id: uuid::Uuid::now_v7().to_string(), kind: kind.into(), title: title.into(), server_id: server_id.map(String::from),
        state: "queued".into(), stage: "Queued".into(), progress: None, log: vec![], error: None, cancellable,
        created_at: now(), started_at: None, ended_at: None,
    };
    let token = CancellationToken::new();
    {
        let mut r = st.ops.lock().unwrap();
        r.items.push_back(op.clone());
        r.tokens.insert(op.id.clone(), token.clone());
        while r.items.len() > 300 {
            if let Some(i) = r.items.iter().position(|o| o.ended_at.is_some()) { let o = r.items.remove(i).unwrap(); r.tokens.remove(&o.id); r.last_emit.remove(&o.id); } else { break; }
        }
    }
    persist_op(st, &op);
    st.emit("op", serde_json::to_value(&op).unwrap());
    let ctx = OpCtx { st: st.clone(), id: op.id.clone(), token };
    tokio::spawn(async move {
        ctx.update(|o| { o.state = "running".into(); o.started_at = Some(now()); o.stage = "Starting".into(); }, true, true);
        // run in its own task so a panic becomes a failed operation instead of one stuck "running" forever
        let res = tokio::spawn(f(ctx.clone())).await.unwrap_or_else(|e| Err(anyhow::anyhow!("internal error: {e}")));
        let cancelled = ctx.cancelled();
        ctx.update(|o| {
            o.ended_at = Some(now());
            match &res {
                Ok(()) => { o.state = "completed".into(); o.progress = Some(1.0); o.stage = "Done".into(); }
                Err(_) if cancelled => { o.state = "cancelled".into(); o.stage = "Cancelled".into(); }
                Err(e) => { o.state = "failed".into(); o.error = Some(format!("{e:#}")); o.stage = "Failed".into(); push_log(o, &format!("error: {e:#}")); }
            }
        }, true, true);
        ctx.st.ops.lock().unwrap().tokens.remove(&ctx.id);
    });
    op
}

pub fn cancel(st: &St, id: &str) -> bool {
    let r = st.ops.lock().unwrap();
    let cancellable = r.items.iter().any(|o| o.id == id && o.cancellable && o.ended_at.is_none());
    match (cancellable, r.tokens.get(id)) {
        (true, Some(t)) => { t.cancel(); true }
        _ => false,
    }
}

pub fn list(st: &St, server_id: Option<&str>, limit: usize) -> Vec<Operation> {
    let r = st.ops.lock().unwrap();
    r.items.iter().rev().filter(|o| server_id.is_none_or(|s| o.server_id.as_deref() == Some(s))).take(limit).cloned().collect()
}

pub fn get(st: &St, id: &str) -> Option<Operation> {
    st.ops.lock().unwrap().items.iter().find(|o| o.id == id).cloned()
}

/// On startup: load recent history and mark anything that was in flight as interrupted.
pub async fn recover(st: &St) -> anyhow::Result<()> {
    let rows = st.db.call(|c| {
        c.execute("UPDATE operations SET state='interrupted', stage='Interrupted by controller restart', ended_at=? WHERE state IN ('queued','running')", [now()])?;
        let mut q = c.prepare("SELECT id,kind,title,server_id,state,stage,progress,log,error,cancellable,created_at,started_at,ended_at FROM operations ORDER BY created_at DESC LIMIT 100")?;
        let v = q.query_map([], |r| Ok(Operation {
            id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, server_id: r.get(3)?, state: r.get(4)?, stage: r.get(5)?, progress: r.get(6)?,
            log: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(), error: r.get(8)?, cancellable: r.get(9)?,
            created_at: r.get(10)?, started_at: r.get(11)?, ended_at: r.get(12)?,
        }))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }).await?;
    let mut r = st.ops.lock().unwrap();
    for op in rows.into_iter().rev() { r.items.push_back(op); }
    Ok(())
}
