//! The individual check rules. Pure: everything comes from `Ctx` and the parsed entries.
use super::{Ctx, Entry, Finding, action};
use crate::jars::{Meta, range::{cmp_str, parse_ver, satisfies}};
use std::collections::HashMap;

/// Entries that would actually be loaded (enabled, readable, with a descriptor for this runtime), indexed by id.
struct Model<'a> { ctx: &'a Ctx, entries: &'a [Entry], loaded: Vec<usize>, by_id: HashMap<String, Vec<usize>>, disabled: HashMap<String, usize> }

fn label(e: &Entry, m: Option<&Meta>) -> String { m.map(|m| m.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| e.title.clone()) }

impl<'a> Model<'a> {
    fn new(ctx: &'a Ctx, entries: &'a [Entry]) -> Self {
        let acc = ctx.accepted();
        let (mut loaded, mut by_id, mut disabled) = (vec![], HashMap::<String, Vec<usize>>::new(), HashMap::new());
        for (i, e) in entries.iter().enumerate() {
            let compatible = e.info.error.is_none() && e.info.metas.iter().any(|m| acc.contains(&m.loader));
            let Some(m) = e.meta(ctx).filter(|_| compatible) else { continue };
            let ids = std::iter::once(&m.id).chain(m.provides.iter());
            if e.enabled { loaded.push(i); for id in ids { by_id.entry(id.to_lowercase()).or_default().push(i); } }
            else { for id in ids { disabled.entry(id.to_lowercase()).or_insert(i); } }
        }
        Model { ctx, entries, loaded, by_id, disabled }
    }

    /// Who provides `id`: (entry index when it is an installed item, known version).
    fn provider(&self, id: &str) -> Option<(Option<usize>, Option<String>)> {
        if let Some(v) = self.ctx.builtin(id) { return Some((None, v)); }
        if let Some(&i) = self.by_id.get(id).and_then(|v| v.first()) { return Some((Some(i), self.entries[i].meta(self.ctx).map(|m| m.version.clone()).filter(|v| !v.is_empty()))); }
        // Fabric API bundles its modules (fabric-api-base, ...) as nested jars that are not listed here
        let fabric_family = matches!(self.ctx.runtime.as_str(), "fabric" | "quilt");
        if fabric_family && id.starts_with("fabric-") && ["fabric-api", "fabric", "quilted_fabric_api"].iter().any(|a| self.by_id.contains_key(*a)) { return Some((None, None)); }
        None
    }
}

pub fn all(ctx: &Ctx, entries: &[Entry]) -> Vec<Finding> {
    let m = Model::new(ctx, entries);
    let mut out = vec![];
    unreadable(&m, &mut out);
    loader_mismatch(&m, &mut out);
    duplicate_ids(&m, &mut out);
    case_duplicates(&m, &mut out);
    for &i in &m.loaded { per_entry(&m, i, &mut out); }
    out
}

fn unreadable(m: &Model, out: &mut Vec<Finding>) {
    for e in m.entries.iter().filter(|e| e.enabled) {
        if let Some(err) = &e.info.error {
            out.push(Finding::new(format!("unreadable:{}", e.item), m.ctx.severity(), Some(&e.item), format!("{} cannot be read", e.filename), "The file is not a valid jar/zip archive, so the server will most likely fail to load it.".into(), vec![err.clone()], action("Remove file", "remove", Some(&e.item))));
        }
    }
}

fn loader_mismatch(m: &Model, out: &mut Vec<Finding>) {
    let acc = m.ctx.accepted();
    for e in m.entries.iter().filter(|e| e.enabled && e.info.error.is_none() && !e.info.metas.is_empty()) {
        if e.info.metas.iter().any(|x| acc.contains(&x.loader)) { continue; }
        let found: Vec<&str> = e.info.metas.iter().map(|x| x.loader).collect();
        let name = label(e, e.info.metas.first());
        out.push(Finding::new(format!("loader:{}", e.item), m.ctx.severity(), Some(&e.item), format!("{name} is built for {}", found.join("/")),
            format!("This server runs {}, which loads {} content. The file will be ignored or cause errors.", m.ctx.runtime, acc.join("/")),
            found.iter().map(|l| format!("{} descriptor found in {}", l, e.filename)).collect(), action("Disable", "toggle", Some(&e.item))));
    }
}

fn duplicate_ids(m: &Model, out: &mut Vec<Finding>) {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for &i in &m.loaded { if let Some(meta) = m.entries[i].meta(m.ctx) { groups.entry(meta.id.to_lowercase()).or_default().push(i); } }
    for (id, mut g) in groups.into_iter().filter(|(_, g)| g.len() > 1) {
        let ver = |i: usize| m.entries[i].meta(m.ctx).map(|x| x.version.clone()).unwrap_or_default();
        g.sort_by(|a, b| cmp_str(&ver(*a), &ver(*b)));
        let oldest = &m.entries[g[0]];
        let mut f = Finding::new(format!("duplicate:{id}"), m.ctx.severity(), Some(&oldest.item), format!("{id} is installed more than once"),
            "Two files declare the same id; only one can load, and some loaders refuse to start.".into(),
            g.iter().map(|&i| format!("{} ({})", m.entries[i].filename, ver(i))).collect(), action(format!("Disable {}", oldest.filename), "toggle", Some(&oldest.item)));
        f.related = g[1..].iter().map(|&i| m.entries[i].item.clone()).collect();
        out.push(f);
    }
}

fn case_duplicates(m: &Model, out: &mut Vec<Finding>) {
    let mut groups: HashMap<String, Vec<&Entry>> = HashMap::new();
    for e in m.entries { groups.entry(e.filename.to_lowercase()).or_default().push(e); }
    for (name, g) in groups.into_iter().filter(|(_, g)| g.len() > 1) {
        let mut f = Finding::new(format!("case:{name}"), "warn", Some(&g[1].item), "File names differ only by case".into(), "On case-insensitive file systems (and in backups restored there) these files overwrite each other.".into(),
            g.iter().map(|e| e.filename.clone()).collect(), action(format!("Remove {}", g[1].filename), "remove", Some(&g[1].item)));
        f.related = vec![g[0].item.clone()];
        out.push(f);
    }
}

fn per_entry(m: &Model, i: usize, out: &mut Vec<Finding>) {
    let (e, ctx) = (&m.entries[i], m.ctx);
    let Some(meta) = e.meta(ctx) else { return };
    let (name, sev) = (label(e, Some(meta)), ctx.severity());
    for d in meta.depends.iter().filter(|d| d.id.to_lowercase() != meta.id.to_lowercase() && d.id != "minecraft") {
        let id = d.id.to_lowercase();
        match m.provider(&id) {
            None if d.required => {
                let act = match m.disabled.get(&id) { Some(&j) => action(format!("Enable {}", m.entries[j].filename), "toggle", Some(&m.entries[j].item)), None => action(format!("Find {}", d.id), "install", None) };
                let mut ev = vec![format!("{} requires {}{}", e.filename, d.id, d.range.as_ref().map(|r| format!(" {r}")).unwrap_or_default())];
                if m.disabled.contains_key(&id) { ev.push(format!("{} is installed but disabled", d.id)); }
                out.push(Finding::new(format!("missing:{}:{id}", e.item), sev, Some(&e.item), format!("{name} needs {}", d.id), format!("{name} requires {} but it is not installed.", d.id), ev, act));
            }
            Some((who, Some(v))) if d.required => {
                if let Some(r) = &d.range && satisfies(r, &v) == Some(false) {
                    let act = who.map(|j| action(format!("Update {}", d.id), "update", Some(&m.entries[j].item))).unwrap_or(None);
                    out.push(Finding::new(format!("range:{}:{id}", e.item), sev, Some(&e.item), format!("{name} needs {} {r}", d.id), format!("{} {v} is installed, which is outside the range {name} accepts.", d.id), vec![format!("{} requires {} {r}", e.filename, d.id), format!("found {} {v}", d.id)], act));
                }
            }
            _ => {}
        }
    }
    for d in meta.depends.iter().filter(|d| d.id == "minecraft") {
        if let Some(r) = &d.range && satisfies(r, &ctx.game_version) == Some(false) { out.push(game_mismatch(ctx, e, &name, format!("{} requires Minecraft {r}", e.filename), format!("{name} does not support Minecraft {}.", ctx.game_version))); }
    }
    if let Some(api) = &meta.api_version && let (Some(a), Some(g)) = (parse_ver(api), parse_ver(&ctx.game_version)) && a.nums.iter().take(2).cmp(g.nums.iter().take(2)) == std::cmp::Ordering::Greater {
        out.push(game_mismatch(ctx, e, &name, format!("{} declares api-version {api}", e.filename), format!("{name} was built for a newer Minecraft API than {}.", ctx.game_version)));
    }
    for b in &meta.breaks {
        let (bid, range) = match b.split_once(' ') { Some((i, r)) => (i, Some(r)), None => (b.as_str(), None) };
        let Some((who, ver)) = m.provider(&bid.to_lowercase()) else { continue };
        if who == Some(i) { continue; }
        let hit = match (range, &ver) { (None, _) => true, (Some(r), Some(v)) => satisfies(r, v) == Some(true), (Some(_), None) => false };
        if !hit { continue; }
        let other = who.map(|j| &m.entries[j]);
        let (act, other_name) = match other { Some(o) => (action(format!("Disable {}", o.filename), "toggle", Some(&o.item)), label(o, o.meta(ctx))), None => (None, bid.to_string()) };
        out.push(Finding::new(format!("breaks:{}:{}", e.item, bid.to_lowercase()), sev, Some(&e.item), format!("{name} is incompatible with {other_name}"), format!("{name} declares that it breaks {b}."), vec![format!("{} declares: breaks {b}", e.filename)], act));
    }
    if ctx.is_mods && meta.environment == "client" {
        out.push(Finding::new(format!("client:{}", e.item), "warn", Some(&e.item), format!("{name} is client-only"), "This mod declares that it only runs on the client. On a server it does nothing and can crash on startup.".into(), vec![format!("{} environment: client", e.filename)], action("Disable", "toggle", Some(&e.item))));
    }
}

fn game_mismatch(ctx: &Ctx, e: &Entry, name: &str, evidence: String, detail: String) -> Finding {
    Finding::new(format!("gamever:{}", e.item), ctx.severity(), Some(&e.item), format!("{name} is not made for Minecraft {}", ctx.game_version), detail, vec![evidence], action("Update", "update", Some(&e.item)))
}
