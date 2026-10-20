use super::{fake::{self, Shared}, *};
use crate::backups::remote::{Store, check_stored};

fn tmp() -> PathBuf { let d = std::env::temp_dir().join(format!("moss-s3-{}", uuid::Uuid::new_v4())); std::fs::create_dir_all(&d).unwrap(); d }
fn data(n: usize) -> Vec<u8> { (0..n).map(|i| (i * 31 % 251) as u8).collect() }
fn sha(b: &[u8]) -> String { hex::encode(Sha256::digest(b)) }

async fn client(url: &str, prefix: &str, secret: &str) -> S3 {
    let cfg = S3Cfg { endpoint: url.into(), region: "us-east-1".into(), bucket: fake::BUCKET.into(), prefix: prefix.into(), path_style: true };
    let mut s = S3::connect(cfg, fake::AK.into(), secret.into(), true).await.unwrap();
    s.part_size = Some(64 << 10);
    s.backoff = Duration::from_millis(1);
    s
}
async fn setup(prefix: &str) -> (S3, Shared, PathBuf) { let (u, f) = fake::start().await; (client(&u, prefix, fake::SK).await, f, tmp()) }
fn file(dir: &Path, name: &str, d: &[u8]) -> PathBuf { let p = dir.join(name); std::fs::write(&p, d).unwrap(); p }

#[tokio::test]
async fn small_files_use_a_single_signed_put_and_round_trip() {
    let (s, f, d) = setup("moss").await;
    let body = data(10_000);
    let p = file(&d, "a.tar.zst", &body);
    s.put(&p, "srv/b1.tar.zst", &sha(&body), &Xfer::new()).await.unwrap();
    {
        let g = f.lock().unwrap();
        assert_eq!(g.log, vec!["PUT /bkt/moss/srv/b1.tar.zst"], "one request, no multipart");
        assert_eq!(g.objects["moss/srv/b1.tar.zst"].data, body);
        assert_eq!(g.sig_failures, 0);
    }
    let info = s.head("srv/b1.tar.zst").await.unwrap().unwrap();
    assert_eq!((info.size, info.sha256.as_deref()), (10_000, Some(sha(&body).as_str())));
    check_stored(&info, 10_000, &sha(&body), &p, Some(64 << 10)).unwrap();
    let out = d.join("got");
    let (h, n) = s.get("srv/b1.tar.zst", &out, &Xfer::new()).await.unwrap();
    assert_eq!((h, n), (sha(&body), 10_000));
    assert_eq!(std::fs::read(&out).unwrap(), body);
    assert!(std::fs::read_dir(&d).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".dl")), "no temp file left");
    s.delete("srv/b1.tar.zst").await.unwrap();
    assert!(s.head("srv/b1.tar.zst").await.unwrap().is_none());
    s.delete("srv/b1.tar.zst").await.unwrap(); // deleting twice is fine
}

#[tokio::test]
async fn big_files_go_up_as_a_multipart_upload_and_verify() {
    let (s, f, d) = setup("").await;
    let body = data(64 * 1024 * 4 + 777);
    let p = file(&d, "big", &body);
    let x = Xfer::new();
    s.put(&p, "srv/big.tar.zst", &sha(&body), &x).await.unwrap();
    assert_eq!(x.done(), body.len() as u64, "progress counts every byte once");
    let g = f.lock().unwrap();
    let log = g.log.join("\n");
    assert!(log.contains("POST /bkt/srv/big.tar.zst?uploads") && log.contains("partNumber=5&uploadId=") && log.contains("POST /bkt/srv/big.tar.zst?uploadId="), "{log}");
    assert_eq!(g.objects["srv/big.tar.zst"].data, body);
    assert_eq!(g.objects["srv/big.tar.zst"].sha.as_deref(), Some(sha(&body).as_str()), "sha256 stored as object metadata");
    assert!(g.objects["srv/big.tar.zst"].etag.ends_with("-5"));
    assert!(g.uploads.is_empty() && g.sig_failures == 0);
    drop(g);
    let info = s.head("srv/big.tar.zst").await.unwrap().unwrap();
    check_stored(&info, body.len() as u64, &sha(&body), &p, Some(64 << 10)).unwrap();
    // checked against the wrong part size the multipart ETag no longer matches
    assert!(check_stored(&info, body.len() as u64, &sha(&body), &p, Some(32 << 10)).is_err());
}

#[tokio::test]
async fn transient_failures_are_retried_and_permanent_ones_are_not() {
    let (s, f, d) = setup("").await;
    let body = data(64 * 1024 * 2 + 5);
    let p = file(&d, "big", &body);
    {
        let mut g = f.lock().unwrap();
        g.inject.push_back(("PUT".into(), 503, "SlowDown".into()));
        g.inject.push_back(("PUT".into(), 500, "InternalError".into()));
        g.inject.push_back(("POST".into(), 500, "InternalError".into()));
    }
    s.put(&p, "k", &sha(&body), &Xfer::new()).await.unwrap();
    let (log, data_ok) = { let g = f.lock().unwrap(); (g.log.clone(), g.objects["k"].data == body) };
    assert!(data_ok);
    assert!(log.iter().filter(|l| l.starts_with("PUT")).count() >= 5, "two failed part uploads were repeated: {log:?}");
    // a 403 is final: exactly one attempt, with a helpful message
    f.lock().unwrap().inject.push_back(("HEAD".into(), 403, "AccessDenied".into()));
    let before = f.lock().unwrap().log.len();
    let e = s.head("k").await.unwrap_err().to_string();
    assert!(e.contains("read, write, list and delete"), "{e}");
    assert_eq!(f.lock().unwrap().log.len(), before + 1);
}

#[tokio::test]
async fn a_failed_multipart_upload_is_aborted() {
    let (mut s, f, d) = setup("").await;
    s.attempts = 2;
    let body = data(64 * 1024 * 3);
    let p = file(&d, "big", &body);
    f.lock().unwrap().always_fail = Some(("PUT".into(), 500, "InternalError".into()));
    let e = s.put(&p, "k", &sha(&body), &Xfer::new()).await.unwrap_err().to_string();
    assert!(e.contains("500") || e.contains("busy"), "{e}");
    let g = f.lock().unwrap();
    assert!(g.log.iter().any(|l| l.starts_with("DELETE /bkt/k?uploadId=")), "abort sent: {:?}", g.log);
    assert!(g.uploads.is_empty() && g.objects.is_empty());
}

#[tokio::test]
async fn cancellation_stops_an_upload() {
    let (s, f, d) = setup("").await;
    let body = data(64 * 1024 * 3);
    let p = file(&d, "big", &body);
    let x = Xfer::new();
    x.token.cancel();
    assert!(s.put(&p, "k", &sha(&body), &x).await.is_err());
    assert!(f.lock().unwrap().objects.is_empty());
}

#[tokio::test]
async fn a_wrong_secret_is_explained_without_leaking_it() {
    let (u, _f) = fake::start().await;
    let s = client(&u, "", "not-the-secret-0000000000000000000000").await;
    let d = tmp();
    let p = file(&d, "a", b"hello");
    let e = s.put(&p, "k", &sha(b"hello"), &Xfer::new()).await.unwrap_err().to_string();
    assert!(e.contains("secret key is wrong"), "{e}");
    assert!(!e.contains("not-the-secret") && !e.contains(fake::AK), "{e}");
    let bad = S3::connect(S3Cfg { endpoint: u.clone(), region: "us-east-1".into(), bucket: fake::BUCKET.into(), prefix: "".into(), path_style: true }, "AKIAWRONG".into(), fake::SK.into(), true).await.unwrap();
    assert!(bad.list("", 1).await.unwrap_err().to_string().contains("access key is not recognised"));
}

#[tokio::test]
async fn corrupted_or_unverifiable_copies_are_rejected() {
    let (s, f, d) = setup("").await;
    let body = data(5000);
    let p = file(&d, "a", &body);
    f.lock().unwrap().corrupt_puts = true;
    s.put(&p, "k", &sha(&body), &Xfer::new()).await.unwrap();
    let info = s.head("k").await.unwrap().unwrap();
    // the stored bytes differ from what was hashed, but the service still reports the original size and checksum;
    // the ETag (an MD5 of what it received) is what exposes tampering in this scenario
    let mut tampered = info.clone();
    tampered.etag = Some("0".repeat(32));
    assert!(check_stored(&tampered, 5000, &sha(&body), &p, None).unwrap_err().to_string().contains("ETag"));
    let mut wrong_size = info.clone(); wrong_size.size = 4999;
    assert!(check_stored(&wrong_size, 5000, &sha(&body), &p, None).unwrap_err().to_string().contains("bytes"));
    let mut wrong_sha = info.clone(); wrong_sha.sha256 = Some("f".repeat(64));
    assert!(check_stored(&wrong_sha, 5000, &sha(&body), &p, None).unwrap_err().to_string().contains("SHA-256"));
    // KMS-encrypted objects have no MD5 ETag: the check is skipped, sha256 still applies
    let mut kms = info.clone(); kms.etag = Some("deadbeef".repeat(4)); kms.kms = true;
    check_stored(&kms, 5000, &sha(&body), &p, None).unwrap();
    // a service that drops our metadata and offers no ETag cannot be verified
    let mut none = info; none.sha256 = None; none.etag = None;
    assert!(check_stored(&none, 5000, &sha(&body), &p, None).is_err());
    // a download of corrupt data is discarded
    let store = Store::S3(Box::new(client(&format!("http://{}", s.base.host_str().unwrap().to_string() + ":" + &s.base.port().unwrap().to_string()), "", fake::SK).await));
    let out = d.join("dl");
    let e = store.get("k", &out, Some((&sha(&body), 5000)), &Xfer::new()).await.unwrap_err().to_string();
    assert!(e.contains("SHA-256"), "{e}");
    assert!(!out.exists() && std::fs::read_dir(&d).unwrap().all(|e| { let n = e.unwrap().file_name().to_string_lossy().to_string(); !n.ends_with(".fetch") && !n.ends_with(".dl") }));
}

#[tokio::test]
async fn listing_is_paginated_and_prefix_aware() {
    let (s, _f, d) = setup("moss").await;
    let p = file(&d, "a", b"x");
    for i in 0..7 { s.put(&p, &format!("srv/{i}.tar.zst"), &sha(b"x"), &Xfer::new()).await.unwrap(); }
    s.put(&p, "other/z", &sha(b"x"), &Xfer::new()).await.unwrap();
    let all = s.list("srv/", 100).await.unwrap();
    assert_eq!(all.len(), 7);
    assert_eq!(all[0].key, "srv/0.tar.zst", "the configured prefix is stripped");
    assert_eq!(s.list("", 100).await.unwrap().len(), 8);
    assert_eq!(s.list("srv/", 3).await.unwrap().len(), 3);
}

#[tokio::test]
async fn private_endpoints_are_refused_unless_allowed() {
    let (u, _f) = fake::start().await;
    let cfg = S3Cfg { endpoint: u, region: "us-east-1".into(), bucket: fake::BUCKET.into(), prefix: "".into(), path_style: true };
    let e = S3::connect(cfg.clone(), "a".into(), "b".into(), false).await.err().unwrap().to_string();
    assert!(e.contains("non-public") && e.contains("MOSS_ALLOW_PRIVATE_BACKUP_HOSTS"), "{e}");
    assert!(S3::connect(cfg, "a".into(), "b".into(), true).await.is_ok());
}

#[tokio::test]
async fn path_style_and_virtual_hosted_urls() {
    let mk = |ps: bool, ep: &str, prefix: &str| {
        let cfg = S3Cfg { endpoint: ep.into(), region: "eu-west-1".into(), bucket: "my-bkt".into(), prefix: prefix.into(), path_style: ps };
        let base = reqwest::Url::parse(ep).unwrap();
        S3 { cfg, access: "a".into(), secret: "b".into(), client: reqwest::Client::new(), base, part_size: None, backoff: Duration::ZERO, attempts: 1 }
    };
    let (u, host, path) = mk(true, "https://s3.example.com", "p").target(Some("srv/a b+c.tar.zst"));
    assert_eq!((u.as_str(), host.as_str(), path.as_str()), ("https://s3.example.com/my-bkt/p/srv/a%20b%2Bc.tar.zst", "s3.example.com", "/my-bkt/p/srv/a%20b%2Bc.tar.zst"));
    let (u, host, path) = mk(false, "https://s3.example.com:8443", "").target(Some("srv/x"));
    assert_eq!((u.as_str(), host.as_str(), path.as_str()), ("https://my-bkt.s3.example.com:8443/srv/x", "my-bkt.s3.example.com:8443", "/srv/x"));
    let (u, _, path) = mk(false, "https://s3.example.com", "").target(None);
    assert_eq!((u.as_str(), path.as_str()), ("https://my-bkt.s3.example.com/", "/"));
    assert_eq!(part_size_for(1 << 30), 16 << 20);
    assert!(part_size_for(5 << 40) * 9000 >= 5 << 40);
}
