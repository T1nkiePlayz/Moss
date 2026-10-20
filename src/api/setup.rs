//! `POST /setup/claim`: the first owner proves control of the host with the setup token.
use super::*;
use std::time::Instant;

#[derive(Deserialize)] pub(super) struct ClaimReq { token: String }

pub(super) async fn setup_claim(State(st): State<St>, who: auth::Claimant, Json(r): Json<ClaimReq>) -> Result<Json<Value>, auth::AuthError> {
    if !auth::setup::pending(&st).await { return Err(ApiError::conflict("setup.not_pending", "this instance already has an owner").into()); }
    let attempt = st.guard.claim.lock().unwrap_or_else(|p| p.into_inner()).hit(&who.id, Instant::now());
    if let Err(retry) = attempt { return Err(auth::AuthError::throttled("setup.rate_limited", retry)); }
    match auth::setup::check_token(&st, &r.token) {
        None => return Err(ApiError::conflict("setup.no_token", "the setup token file is missing or damaged; restart Moss to create a new one").into()),
        Some(false) => return Err(ApiError::new(StatusCode::FORBIDDEN, "setup.bad_token", "that is not the setup token").into()),
        Some(true) => {}
    }
    let (id, email) = (who.id.clone(), who.email.clone());
    let outcome = st.db.call(move |c| auth::setup::claim_db(c, &id, &email)).await.map_err(ApiError::from)?;
    if outcome == auth::setup::Claim::AlreadyClaimed { return Err(ApiError::conflict("setup.not_pending", "this instance already has an owner").into()); }
    if let Err(e) = std::fs::remove_file(auth::setup::token_path(&st)) { tracing::warn!("could not delete the setup token file: {e}"); }
    auth::drop_cached(&st, &who.id);
    st.audit(&who.email, "setup.claim", "owner", "", None).await;
    Ok(Json(json!({"role": "owner"})))
}
