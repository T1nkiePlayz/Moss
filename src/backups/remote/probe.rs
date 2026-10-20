//! "Test destination": write a small object, read it back, list, delete it. Reports each step in plain words.
use super::{Store, Xfer};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

pub async fn run(store: &Store, tmp: &Path) -> (bool, Vec<Value>) {
    let mut steps: Vec<Value> = vec![];
    let mut step = |name: &str, r: Result<String, String>| -> bool { let ok = r.is_ok(); steps.push(json!({"step": name, "ok": ok, "detail": r.unwrap_or_else(|e| e)})); ok };
    let id = uuid::Uuid::new_v4();
    let key = format!("moss-test/{id}.txt");
    let body = format!("Moss destination test {id}\n");
    let sha = hex::encode(Sha256::digest(body.as_bytes()));
    let (src, back) = (tmp.join(format!("probe-{id}.src")), tmp.join(format!("probe-{id}.back")));
    let mut ok = true;
    if tokio::fs::write(&src, &body).await.is_err() { step("Write a test file", Err("could not create a scratch file in Moss's temp folder".into())); return (false, steps); }
    let x = Xfer::new();
    ok &= step("Write a test file", store.put(&src, &key, &sha, &x).await.map(|_| "uploaded a small file".into()).map_err(|e| format!("{e:#}")));
    if ok {
        let r = async {
            let info = store.head(&key).await.map_err(|e| format!("{e:#}"))?.ok_or("the file is not there after writing it")?;
            if info.size != body.len() as u64 { return Err(format!("the file has {} bytes, expected {}", info.size, body.len())); }
            store.get(&key, &back, Some((&sha, body.len() as u64)), &x).await.map_err(|e| format!("{e:#}"))?;
            Ok("read it back and the contents match".to_string())
        }.await;
        ok &= step("Read it back", r);
        ok &= step("List files", store.list("moss-test/", 50).await.map_err(|e| format!("{e:#}")).and_then(|v| if v.iter().any(|o| o.key == key) { Ok("the test file is listed".into()) } else { Err("the test file is missing from the listing".into()) }));
    }
    // always try to clean up, even after a failure
    let del = store.delete(&key).await.map_err(|e| format!("{e:#}"));
    let gone = match &del { Ok(()) => store.head(&key).await.map(|h| h.is_none()).unwrap_or(true), Err(_) => false };
    ok &= step("Delete it", del.and_then(|_| if gone { Ok("removed the test file".to_string()) } else { Err("the file is still there after deleting it".into()) }));
    let _ = tokio::fs::remove_file(&src).await;
    let _ = tokio::fs::remove_file(&back).await;
    (ok, steps)
}
