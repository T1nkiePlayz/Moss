//! Turns verified claims into a local `User`: cached lookups, invited-account linking and first-owner bootstrap.
use super::{User, jwt::Claims};
use crate::{error::{ApiError, ApiResult}, state::St};
use std::time::{Duration, Instant};

pub async fn provision(st: &St, c: Claims) -> ApiResult<User> {
    if let Some((t, u)) = st.users.lock().unwrap().get(&c.sub)
        && t.elapsed() < Duration::from_secs(30) { return Ok(u.clone()); }
    let email = c.email.clone().unwrap_or_default().to_lowercase();
    let verified = c.user_metadata["email_verified"].as_bool().unwrap_or(false);
    let owners = st.cfg.owner_email_list();
    let require_verified = st.cfg.require_verified_email;
    let sub = c.sub.clone();
    let user = st.db.call(move |db| {
        let by_id = db.query_row("SELECT id,email,role FROM users WHERE id=?", [&sub], |r| Ok(User { id: r.get(0)?, email: r.get(1)?, role: r.get(2)?, token: None }));
        if let Ok(u) = by_id { return Ok(Some(u)); }
        if email.is_empty() || (require_verified && !verified) { return Ok(None); }
        // invited (pre-created, unlinked) account?
        if let Ok((old_id, role)) = db.query_row("SELECT id,role FROM users WHERE email=? AND linked=0", [&email], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            // grants cascade on user delete, so free the unique email, add the real row, move grants, then drop the placeholder
            db.execute("UPDATE users SET email=email||'#invite' WHERE id=?", [&old_id])?;
            db.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,?,1,?)", rusqlite::params![sub, email, role, crate::db::now()])?;
            db.execute("UPDATE grants SET user_id=? WHERE user_id=?", [&sub, &old_id])?;
            db.execute("DELETE FROM users WHERE id=?", [&old_id])?;
            return Ok(Some(User { id: sub, email, role, token: None }));
        }
        let owner_count: i64 = db.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get(0))?;
        let role = (owners.contains(&email) || (owners.is_empty() && owner_count == 0)).then_some("owner");
        match role {
            Some(role) => {
                db.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,?,1,?)", rusqlite::params![sub, email, role, crate::db::now()])?;
                Ok(Some(User { id: sub, email, role: role.into(), token: None }))
            }
            None => Ok(None),
        }
    }).await?;
    match user {
        Some(u) => {
            st.users.lock().unwrap().insert(c.sub, (Instant::now(), u.clone()));
            Ok(u)
        }
        None => Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "auth.not_provisioned", "This account has not been invited to this Moss instance (or its email is not verified yet).")),
    }
}

pub fn drop_cached(st: &St, id: &str) { st.users.lock().unwrap().remove(id); }
