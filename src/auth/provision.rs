//! Turns verified claims into a local `User`: cached lookups, invited-account linking and owner-email bootstrap.
//! There is deliberately no "first account becomes owner" path: with no owner configured, ownership is claimed
//! with the setup token (see `setup`).
use super::{User, jwt::Claims, setup};
use crate::{error::{ApiError, ApiResult}, state::St};
use axum::http::StatusCode;
use std::{sync::atomic::{AtomicU64, Ordering}, time::{Duration, Instant}};

const CACHE_TTL: Duration = Duration::from_secs(30);

/// Bumped whenever a cached user is dropped. A lookup that raced with a drop (it read the database before
/// the change but would insert after the drop) sees the epoch moved and does not cache its stale result.
static EPOCH: AtomicU64 = AtomicU64::new(0);

enum Found { User(User), SetupPending, No }

pub async fn provision(st: &St, c: &Claims) -> ApiResult<User> {
    if let Some((t, u)) = st.users.lock().unwrap_or_else(|p| p.into_inner()).get(&c.sub)
        && t.elapsed() < CACHE_TTL { return Ok(u.clone()); }
    let epoch = EPOCH.load(Ordering::SeqCst);
    let email = c.email.clone().unwrap_or_default().to_lowercase();
    let verified = c.email_verified();
    let owners = st.cfg.owner_email_list();
    let require_verified = st.cfg.require_verified_email;
    let sub = c.sub.clone();
    let found = st.db.call(move |db| {
        let by_id = db.query_row("SELECT id,email,role FROM users WHERE id=?", [&sub], |r| Ok(User { id: r.get(0)?, email: r.get(1)?, role: r.get(2)?, token: None, via_token: None, aal: None }));
        if let Ok(u) = by_id { return Ok(Found::User(u)); }
        if email.is_empty() || (require_verified && !verified) { return Ok(Found::No); }
        // invited (pre-created, unlinked) account?
        if let Ok((old_id, role)) = db.query_row("SELECT id,role FROM users WHERE email=? AND linked=0", [&email], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            // grants cascade on user delete, so free the unique email, add the real row, move grants, then drop the placeholder
            let tx = db.unchecked_transaction()?;
            tx.execute("UPDATE users SET email=email||'#invite' WHERE id=?", [&old_id])?;
            tx.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,?,1,?)", rusqlite::params![sub, email, role, crate::db::now()])?;
            tx.execute("UPDATE grants SET user_id=? WHERE user_id=?", [&sub, &old_id])?;
            tx.execute("DELETE FROM users WHERE id=?", [&old_id])?;
            tx.commit()?;
            return Ok(Found::User(User { id: sub, email, role, token: None, via_token: None, aal: None }));
        }
        if owners.contains(&email) {
            db.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,'owner',1,?)", rusqlite::params![sub, email, crate::db::now()])?;
            return Ok(Found::User(User { id: sub, email, role: "owner".into(), token: None, via_token: None, aal: None }));
        }
        if setup::pending_db(db, !owners.is_empty())? { return Ok(Found::SetupPending); }
        Ok(Found::No)
    }).await?;
    match found {
        Found::User(u) => {
            if EPOCH.load(Ordering::SeqCst) == epoch { st.users.lock().unwrap_or_else(|p| p.into_inner()).insert(c.sub.clone(), (Instant::now(), u.clone())); }
            Ok(u)
        }
        Found::SetupPending => Err(ApiError::new(StatusCode::FORBIDDEN, "auth.setup_required", "This Moss instance has no owner yet. Enter the setup token from the host to claim it.")),
        Found::No => Err(ApiError::new(StatusCode::FORBIDDEN, "auth.not_provisioned", "This account has not been invited to this Moss instance (or its email is not verified yet).")),
    }
}

/// Forget one user's cached identity (role change, removal).
pub fn drop_cached(st: &St, id: &str) {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    st.users.lock().unwrap_or_else(|p| p.into_inner()).remove(id);
}

/// Bumped whenever a user or grant changes; long-lived streams compare it to drop authorization they captured earlier.
pub fn epoch() -> u64 { EPOCH.load(Ordering::SeqCst) }

/// Forget every cached identity (grant changes).
pub fn drop_all(st: &St) {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    st.users.lock().unwrap_or_else(|p| p.into_inner()).clear();
}
