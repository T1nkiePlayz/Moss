//! Enable/disable a content file by renaming `<name>.jar` <-> `<name>.jar.disabled` (atomically, under the per-server lock).
use super::*;

#[derive(Deserialize)] pub struct ToggleReq { enabled: bool }

/// The file name after a toggle, or `None` when the file is already in the requested state.
pub(super) fn toggled_name(name: &str, enable: bool) -> Option<String> {
    match (name.strip_suffix(".disabled"), enable) { (Some(base), true) => Some(base.to_string()), (None, false) => Some(format!("{name}.disabled")), _ => None }
}

/// Renames `from` to `to` without ever replacing an existing file.
pub(super) fn rename_new(from: &std::path::Path, to: &std::path::Path) -> ApiResult<()> {
    if !from.is_file() { return Err(ApiError::not_found("content file")); }
    if std::fs::symlink_metadata(to).is_ok() { return Err(ApiError::conflict("content.exists", "a file with the target name already exists")); }
    std::fs::rename(from, to).map_err(|e| ApiError::from(anyhow!(e)))
}

pub async fn toggle(State(st): State<St>, user: User, Path((id, item)): Path<(String, String)>, Json(r): Json<ToggleReq>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let _lock = st.lock_for(&s.id).try_lock_owned().map_err(|_| ApiError::conflict("server.locked", "another operation is changing this server"))?;
    let (row, filename, dir) = if let Some(f) = item.strip_prefix("manual:") {
        (None, content_filename(f, c.ext).map_err(|e| ApiError::bad("validation", e.to_string()))?, c.dir.clone())
    } else {
        let (i, sid) = (item.clone(), s.id.clone());
        let found: Option<(String, String)> = st.db.call(move |db| db.query_row("SELECT filename,dir FROM content_items WHERE id=? AND server_id=?", [i, sid], |r| Ok((r.get(0)?, r.get(1)?))).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await?;
        let (f, d) = found.ok_or_else(|| ApiError::not_found("content item"))?;
        (Some(item.clone()), content_filename(&f, c.ext).map_err(|e| ApiError::bad("validation", e.to_string()))?, d)
    };
    let new_id = if let Some(new) = toggled_name(&filename, r.enabled) {
        let root = s.dir(&st);
        let (from, to) = (safe_join(&root, &format!("{dir}/{filename}"))?, safe_join(&root, &format!("{dir}/{new}"))?);
        tokio::task::spawn_blocking(move || rename_new(&from, &to)).await.map_err(|e| anyhow!(e))??;
        if let Some(rid) = &row {
            let (rid, new2) = (rid.clone(), new.clone());
            st.db.call(move |db| db.execute("UPDATE content_items SET filename=? WHERE id=?", rusqlite::params![new2, rid])).await?;
        }
        st.audit(&user.actor(), if r.enabled { "content.enable" } else { "content.disable" }, &s.name, &filename, Some(&s.id)).await;
        row.clone().unwrap_or_else(|| format!("manual:{new}"))
    } else { row.clone().unwrap_or_else(|| item.clone()) };
    let items = items_json(&st, &s, &c).await?;
    let found = items.into_iter().find(|i| i["id"] == new_id.as_str()).ok_or_else(|| ApiError::not_found("content item"))?;
    Ok(Json(json!({"item": found, "requires_restart": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_flip_both_ways_and_noop_when_already_there() {
        assert_eq!(toggled_name("a.jar", false).as_deref(), Some("a.jar.disabled"));
        assert_eq!(toggled_name("a.jar.disabled", true).as_deref(), Some("a.jar"));
        assert_eq!(toggled_name("a.jar", true), None);
        assert_eq!(toggled_name("a.jar.disabled", false), None);
    }

    #[test]
    fn filenames_reject_paths_and_foreign_extensions() {
        for ok in ["a.jar", "a.jar.disabled", "A B.JAR"] { assert!(content_filename(ok, "jar").is_ok(), "{ok}"); }
        for bad in ["../a.jar", "x/a.jar", ".a.jar", "a.zip", "a.jar.disabled.exe", "", "a.disabled"] { assert!(content_filename(bad, "jar").is_err(), "{bad}"); }
    }

    #[test]
    fn rename_never_overwrites_and_requires_the_source() {
        let d = std::env::temp_dir().join(format!("moss-toggle-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("a.jar"), d.join("a.jar.disabled"));
        assert!(rename_new(&a, &b).is_err()); // missing source
        std::fs::write(&a, "one").unwrap();
        rename_new(&a, &b).unwrap();
        assert!(!a.exists() && std::fs::read_to_string(&b).unwrap() == "one");
        std::fs::write(&a, "two").unwrap();
        assert!(rename_new(&a, &b).is_err()); // target exists: nothing is lost
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "two");
        let _ = std::fs::remove_dir_all(&d);
    }
}
