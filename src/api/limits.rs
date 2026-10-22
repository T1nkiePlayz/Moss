//! Validation and persistence of per-server resource limits (`memory_limit_mb`, `cpu_limit_pct`, `pids_limit`).
use super::*;
use crate::platform::cgroup;
use serde::Deserializer;

/// Distinguishes a missing JSON field (leave alone) from an explicit `null` (clear).
pub(super) fn double_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// New values for the three limits; the outer `Option` is "was the field sent", the inner one is the value or `null`.
#[derive(Default, Clone, Copy)]
pub(super) struct LimitsPatch { pub memory: Option<Option<i64>>, pub cpu: Option<Option<i64>>, pub pids: Option<Option<i64>> }

impl LimitsPatch {
    pub fn is_empty(&self) -> bool { self.memory.is_none() && self.cpu.is_none() && self.pids.is_none() }
}

pub(super) fn validate(p: &LimitsPatch, cores: i64) -> ApiResult<()> {
    let check = |v: Option<Option<i64>>, lo: i64, hi: i64, field: &str| -> ApiResult<()> {
        match v.flatten() {
            Some(n) if !(lo..=hi).contains(&n) => Err(ApiError::bad("validation", format!("{field} out of range")).field(field, &format!("{lo}–{hi}"))),
            _ => Ok(()),
        }
    };
    check(p.memory, 256, 262_144, "memory_limit_mb")?;
    check(p.cpu, 10, 100 * cores.max(1), "cpu_limit_pct")?;
    check(p.pids, 64, 32_768, "pids_limit")
}

/// Write the sent fields (limits take effect the next time the server starts).
pub(super) async fn save(st: &St, id: &str, p: LimitsPatch) -> ApiResult<()> {
    if p.is_empty() { return Ok(()); }
    let i = id.to_string();
    st.db.call(move |c| {
        if let Some(v) = p.memory { c.execute("UPDATE servers SET memory_limit_mb=? WHERE id=?", rusqlite::params![v, i])?; }
        if let Some(v) = p.cpu { c.execute("UPDATE servers SET cpu_limit_pct=? WHERE id=?", rusqlite::params![v, i])?; }
        if let Some(v) = p.pids { c.execute("UPDATE servers SET pids_limit=? WHERE id=?", rusqlite::params![v, i])?; }
        Ok(())
    }).await?;
    Ok(())
}

/// `current` is the stored memory limit: an unchanged value is never re-checked against the host cap.
pub(super) fn validate_now(p: &LimitsPatch, current: Option<i64>) -> ApiResult<()> {
    validate(p, cgroup::cores())?;
    if let Some(Some(m)) = p.memory && Some(m) != current && let Some(msg) = crate::platform::memory_over(m, crate::platform::host_memory_mb()) { return Err(ApiError::bad("validation", "memory_limit_mb out of range").field("memory_limit_mb", &msg)); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_cap_applies_only_to_changed_memory_limit() {
        let over = crate::platform::host_memory_mb().max(256) + 1;
        let p = LimitsPatch { memory: Some(Some(over)), ..Default::default() };
        assert!(validate_now(&p, None).is_err(), "a new value above the host is refused");
        assert!(validate_now(&p, Some(over)).is_ok(), "an unchanged value above the host is left alone");
    }

    #[test]
    fn bounds_and_null() {
        let ok = |p: LimitsPatch| validate(&p, 4).is_ok();
        assert!(ok(LimitsPatch::default()));
        assert!(ok(LimitsPatch { memory: Some(None), cpu: Some(None), pids: Some(None) }));
        assert!(ok(LimitsPatch { memory: Some(Some(256)), cpu: Some(Some(400)), pids: Some(Some(64)) }));
        assert!(!ok(LimitsPatch { memory: Some(Some(255)), ..Default::default() }));
        assert!(!ok(LimitsPatch { cpu: Some(Some(401)), ..Default::default() }));
        assert!(!ok(LimitsPatch { cpu: Some(Some(9)), ..Default::default() }));
        assert!(!ok(LimitsPatch { pids: Some(Some(32_769)), ..Default::default() }));
        assert!(!ok(LimitsPatch { pids: Some(Some(63)), ..Default::default() }));
    }

    #[test]
    fn double_option_distinguishes_null_from_missing() {
        #[derive(Deserialize)]
        struct T { #[serde(default, deserialize_with = "double_option")] v: Option<Option<i64>> }
        assert_eq!(serde_json::from_str::<T>("{}").unwrap().v, None);
        assert_eq!(serde_json::from_str::<T>(r#"{"v":null}"#).unwrap().v, Some(None));
        assert_eq!(serde_json::from_str::<T>(r#"{"v":5}"#).unwrap().v, Some(Some(5)));
    }
}
