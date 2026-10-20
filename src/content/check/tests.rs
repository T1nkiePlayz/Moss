//! Rule tests: table-driven over hand-built entries; one test goes through real in-memory jars end to end.
use super::*;
use crate::jars::Dep;

fn ctx(runtime: &str, gv: &str) -> Ctx {
    Ctx { runtime: runtime.into(), game_version: gv.into(), loader_version: Some("0.16.0".into()), java_major: 21, is_mods: matches!(runtime, "fabric" | "quilt" | "forge" | "neoforge") }
}

fn mk(loader: &'static str, id: &str, ver: &str, deps: &[(&str, bool, Option<&str>)]) -> Meta {
    let mut m = Meta::new(loader, id);
    m.version = ver.into();
    m.depends = deps.iter().map(|(i, r, rg)| Dep { id: i.to_string(), required: *r, range: rg.map(String::from) }).collect();
    m
}

fn entry(file: &str, enabled: bool, metas: Vec<Meta>) -> Entry {
    Entry { item: format!("manual:{file}"), title: file.into(), filename: file.into(), enabled, info: Arc::new(Info { metas, error: None }) }
}

fn ids(fs: &[Finding]) -> Vec<String> { let mut v: Vec<String> = fs.iter().map(|f| f.id.clone()).collect(); v.sort(); v }

struct Case { name: &'static str, ctx: Ctx, entries: Vec<Entry>, want: Vec<&'static str> }

#[test]
fn rules_table() {
    let fab = || ctx("fabric", "1.21.1");
    let cases = vec![
        Case { name: "clean", ctx: fab(), want: vec![], entries: vec![
            entry("a.jar", true, vec![mk("fabric", "a", "1.0", &[("fabricloader", true, Some(">=0.15")), ("minecraft", true, Some("~1.21")), ("java", true, Some(">=21")), ("fabric-api", true, None), ("fabric-resource-loader-v0", true, None)])]),
            entry("fabric-api.jar", true, vec![mk("fabric", "fabric-api", "0.100.0", &[])])] },
        Case { name: "missing required, optional ignored", ctx: fab(), want: vec!["missing:manual:a.jar:lib"], entries: vec![
            entry("a.jar", true, vec![mk("fabric", "a", "1", &[("lib", true, None), ("opt", false, None)])])] },
        Case { name: "dependency present but disabled", ctx: fab(), want: vec!["missing:manual:a.jar:lib"], entries: vec![
            entry("a.jar", true, vec![mk("fabric", "a", "1", &[("lib", true, None)])]), entry("lib.jar.disabled", false, vec![mk("fabric", "lib", "1", &[])])] },
        Case { name: "range violation on a mod", ctx: fab(), want: vec!["range:manual:a.jar:lib"], entries: vec![
            entry("a.jar", true, vec![mk("fabric", "a", "1", &[("lib", true, Some(">=2.0"))])]), entry("lib.jar", true, vec![mk("fabric", "lib", "1.5", &[])])] },
        Case { name: "range violation on a built-in (java)", ctx: fab(), want: vec!["range:manual:a.jar:java"], entries: vec![
            entry("a.jar", true, vec![mk("fabric", "a", "1", &[("java", true, Some(">=22"))])])] },
        Case { name: "duplicate ids", ctx: fab(), want: vec!["duplicate:a"], entries: vec![
            entry("a1.jar", true, vec![mk("fabric", "a", "1", &[])]), entry("a2.jar", true, vec![mk("fabric", "a", "2", &[])])] },
        Case { name: "breaks present, with and without range", ctx: fab(), want: vec!["breaks:manual:a.jar:b"], entries: {
            let mut a = mk("fabric", "a", "1", &[]);
            a.breaks = vec!["b <2".into(), "c <1".into()];
            vec![entry("a.jar", true, vec![a]), entry("b.jar", true, vec![mk("fabric", "b", "1.9", &[])]), entry("c.jar", true, vec![mk("fabric", "c", "3", &[])])] } },
        Case { name: "fabric mod on a forge server", ctx: ctx("forge", "1.20.1"), want: vec!["loader:manual:a.jar"], entries: vec![entry("a.jar", true, vec![mk("fabric", "a", "1", &[])])] },
        Case { name: "plugin.yml in mods", ctx: fab(), want: vec!["loader:manual:p.jar"], entries: vec![entry("p.jar", true, vec![mk("bukkit", "P", "1", &[])])] },
        Case { name: "multi-loader jar is fine on either", ctx: ctx("forge", "1.20.1"), want: vec![], entries: vec![entry("a.jar", true, vec![mk("fabric", "a", "1", &[]), mk("forge", "a", "1", &[])])] },
        Case { name: "quilt accepts fabric mods", ctx: ctx("quilt", "1.21.1"), want: vec![], entries: vec![entry("a.jar", true, vec![mk("fabric", "a", "1", &[("fabricloader", true, Some(">=0.15"))])])] },
        Case { name: "game version mismatch", ctx: fab(), want: vec!["gamever:manual:a.jar"], entries: vec![entry("a.jar", true, vec![mk("fabric", "a", "1", &[("minecraft", true, Some("1.20.x"))])])] },
        Case { name: "plugin api-version newer than server", ctx: ctx("paper", "1.20.4"), want: vec!["gamever:manual:p.jar"], entries: {
            let mut m = mk("bukkit", "P", "1", &[]); m.api_version = Some("1.21".into()); vec![entry("p.jar", true, vec![m])] } },
        Case { name: "plugin api-version older is fine", ctx: ctx("paper", "1.21.1"), want: vec![], entries: {
            let mut m = mk("bukkit", "P", "1", &[]); m.api_version = Some("1.13".into()); vec![entry("p.jar", true, vec![m])] } },
        Case { name: "client-only mod", ctx: fab(), want: vec!["client:manual:a.jar"], entries: {
            let mut m = mk("fabric", "a", "1", &[]); m.environment = "client"; vec![entry("a.jar", true, vec![m])] } },
        Case { name: "plugin names are case-insensitive", ctx: ctx("paper", "1.21.1"), want: vec![], entries: vec![
            entry("a.jar", true, vec![mk("bukkit", "A", "1", &[("vault", true, None), ("Paper", true, None)])]), entry("v.jar", true, vec![mk("bukkit", "Vault", "1", &[])])] },
        Case { name: "case-only duplicate file names", ctx: fab(), want: vec!["case:a.jar"], entries: vec![
            entry("a.jar", true, vec![mk("fabric", "a", "1", &[])]), entry("A.jar", true, vec![mk("fabric", "a2", "1", &[])])] },
        Case { name: "forge version range and neoforge built-in", ctx: ctx("neoforge", "1.21.1"), want: vec!["range:manual:a.jar:neoforge"], entries: vec![
            entry("a.jar", true, vec![mk("neoforge", "a", "1", &[("neoforge", true, Some("[22,)")), ("minecraft", true, Some("[1.21,1.22)"))])])] },
        Case { name: "disabled items are not checked", ctx: fab(), want: vec![], entries: vec![entry("a.jar.disabled", false, vec![mk("fabric", "a", "1", &[("missing", true, None)])])] },
        Case { name: "manual jar with no descriptor is ignored", ctx: fab(), want: vec![], entries: vec![entry("lib.jar", true, vec![])] },
    ];
    for mut c in cases {
        if c.name.contains("neoforge built-in") { c.ctx.loader_version = Some("21.1.5".into()); }
        let got = ids(&run(&c.ctx, &c.entries));
        assert_eq!(got, c.want.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{}", c.name);
    }
}

#[test]
fn unreadable_jars_are_reported_and_skipped() {
    let mut e = entry("bad.jar", true, vec![]);
    e.info = Arc::new(Info { metas: vec![], error: Some("not a zip".into()) });
    let f = run(&ctx("fabric", "1.21.1"), &[e]);
    assert_eq!(ids(&f), vec!["unreadable:manual:bad.jar"]);
    assert_eq!(f[0].severity, "critical");
    assert_eq!(f[0].action.as_ref().unwrap().kind, "remove");
}

#[test]
fn severity_depends_on_content_kind_and_actions_point_at_a_fix() {
    let es = vec![entry("a.jar", true, vec![mk("bukkit", "A", "1", &[("Vault", true, None)])]), entry("Vault.jar.disabled", false, vec![mk("bukkit", "Vault", "1", &[])])];
    let f = run(&ctx("paper", "1.21.1"), &es);
    assert_eq!((f[0].severity, f[0].action.as_ref().unwrap().kind, f[0].action.as_ref().unwrap().item.as_deref()), ("warn", "toggle", Some("manual:Vault.jar.disabled")));
    let f = run(&ctx("fabric", "1.21.1"), &[entry("a.jar", true, vec![mk("fabric", "a", "1", &[("lib", true, None)])])]);
    assert_eq!((f[0].severity, f[0].action.as_ref().unwrap().kind), ("critical", "install"));
}

#[test]
fn duplicate_suggests_disabling_the_older_and_marks_both_files() {
    let f = run(&ctx("fabric", "1.21.1"), &[entry("a-2.jar", true, vec![mk("fabric", "a", "1.10", &[])]), entry("a-1.jar", true, vec![mk("fabric", "a", "1.9", &[])])]);
    assert_eq!(f[0].item.as_deref(), Some("manual:a-1.jar"));
    assert_eq!(f[0].related, vec!["manual:a-2.jar"]);
}

#[test]
fn plan_conflicts_only_report_new_problems() {
    let c = ctx("fabric", "1.21.1");
    let before = vec![entry("lib.jar", true, vec![mk("fabric", "lib", "1.5", &[])]), entry("a.jar", true, vec![mk("fabric", "a", "1", &[("lib", true, Some("<2"))])]),
        entry("old.jar", true, vec![mk("fabric", "old", "1", &[("gone", true, None)])])];
    assert!(run(&c, &before).iter().any(|f| f.id == "missing:manual:old.jar:gone")); // pre-existing, must not be blamed on the plan
    let out = conflicts(&c, &before, &[Change { replaces: Some("manual:lib.jar".into()), new_version: "2.1".into() }]);
    assert_eq!(ids(&out), vec!["range:manual:a.jar:lib"]);
    assert!(conflicts(&c, &before, &[Change { replaces: Some("manual:lib.jar".into()), new_version: "1.9".into() }]).is_empty());
}

#[test]
fn end_to_end_from_real_jar_bytes() {
    use std::io::Cursor;
    let dep = r#"{"id":"b","version":"1.0.0"}"#;
    let a = r#"{"id":"a","version":"1","depends":{"b":">=2.0.0","c":"*"}}"#;
    let infos: Vec<Arc<Info>> = [("fabric.mod.json", a), ("fabric.mod.json", dep)].iter().map(|(n, c)| Arc::new(crate::jars::parse(Cursor::new(crate::jars::tests::make_zip(&[(*n, *c)])), "x"))).collect();
    let es: Vec<Entry> = infos.into_iter().zip(["a.jar", "b.jar"]).map(|(info, f)| Entry { item: format!("manual:{f}"), title: f.into(), filename: f.into(), enabled: true, info }).collect();
    assert_eq!(ids(&run(&ctx("fabric", "1.21.1"), &es)), vec!["missing:manual:a.jar:c", "range:manual:a.jar:b"]);
}
