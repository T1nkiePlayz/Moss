//! Remote-admin setup: the generated `rcon_password` in `moss-game.cfg`, and the config-file edits Palworld and Zomboid
//! need before launch (their RCON is enabled in their own settings files, not on the command line).
use super::{Game, SETTINGS_FILE, admin, install::set_properties, read_settings};
use std::{collections::HashMap, io::Write, path::Path};

/// Returns the server's RCON password, generating and appending one to `moss-game.cfg` when missing. `None` for games
/// without RCON.
pub fn ensure_password(dir: &Path, g: &Game) -> std::io::Result<Option<String>> {
    if admin::flavor(g).is_none() { return Ok(None); }
    if let Some(p) = read_settings(dir).remove("rcon_password").filter(|p| !p.is_empty()) { return Ok(Some(p)); }
    let pw = crate::auth::secret::random_string(20);
    let path = dir.join(SETTINGS_FILE);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let head = if old.is_empty() { format!("# Moss settings for this {} server. Edit and restart to apply.\n", g.name) } else if old.ends_with('\n') { String::new() } else { "\n".into() };
    writeln!(f, "{head}rcon_password={pw}")?;
    Ok(Some(pw))
}

/// Splits the inside of `OptionSettings=(...)` into top-level `Key=Value` items (commas inside quotes or parentheses stay).
fn split_items(body: &str) -> Vec<String> {
    let (mut items, mut cur, mut depth, mut quoted) = (vec![], String::new(), 0i32, false);
    for c in body.chars() {
        match c {
            '"' => { quoted = !quoted; cur.push(c); }
            '(' if !quoted => { depth += 1; cur.push(c); }
            ')' if !quoted => { depth -= 1; cur.push(c); }
            ',' if !quoted && depth == 0 => items.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() { items.push(cur); }
    items
}

/// Sets RCON in a `PalWorldSettings.ini` (every other option is kept as is).
pub fn patch_palworld(text: &str, port: u16, pw: &str) -> String {
    let kv = [("RCONEnabled", "True".to_string()), ("RCONPort", port.to_string()), ("AdminPassword", format!("\"{pw}\""))];
    const OPEN: &str = "OptionSettings=(";
    let Some(at) = text.find(OPEN) else {
        let items: Vec<String> = kv.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
        let head = if text.contains("[/Script/Pal.PalGameWorldSettings]") { "" } else { "[/Script/Pal.PalGameWorldSettings]\n" };
        return format!("{text}{sep}{head}{OPEN}{})\n", items.join(","));
    };
    let start = at + OPEN.len();
    let (mut depth, mut quoted, mut end) = (1i32, false, text.len());
    for (i, c) in text[start..].char_indices() {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => { depth -= 1; if depth == 0 { end = start + i; break; } }
            _ => {}
        }
    }
    let mut items = split_items(&text[start..end]);
    for (k, v) in &kv {
        let new = format!("{k}={v}");
        match items.iter_mut().find(|i| i.trim_start().split('=').next().is_some_and(|n| n.trim() == *k)) { Some(i) => *i = new, None => items.push(new) }
    }
    format!("{}{}{}", &text[..start], items.join(","), &text[end..])
}

fn write_if_changed(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|t| t == text) { return Ok(()); }
    if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
    std::fs::write(path, text)
}

/// The non-empty `AdminPassword` already set in a `PalWorldSettings.ini` (Palworld's RCON uses it).
pub fn palworld_admin_password(text: &str) -> Option<String> {
    let at = text.find("OptionSettings=(")? + "OptionSettings=(".len();
    split_items(&text[at..]).into_iter().find_map(|i| {
        let (k, v) = i.split_once('=')?;
        (k.trim() == "AdminPassword").then(|| v.trim().trim_end_matches(')').trim_matches('"').to_string()).filter(|v| !v.is_empty())
    })
}

/// Patches RCON into Palworld's settings. An AdminPassword the operator set (Config tab) wins: it becomes the RCON
/// password in `moss-game.cfg` instead of being overwritten. Returns the password in use.
fn apply_palworld(dir: &Path, port: u16, pw: &str) -> std::io::Result<String> {
    let path = dir.join("Pal/Saved/Config/LinuxServer/PalWorldSettings.ini");
    let base = std::fs::read_to_string(&path).or_else(|_| std::fs::read_to_string(dir.join("DefaultPalWorldSettings.ini"))).unwrap_or_default();
    let pw = match palworld_admin_password(&base) {
        Some(own) if own != pw && admin::safe_text(&own, 128) => {
            let cfg = dir.join(SETTINGS_FILE);
            std::fs::write(&cfg, set_properties(&std::fs::read_to_string(&cfg).unwrap_or_default(), &[("rcon_password", own.clone())]))?;
            own
        }
        _ => pw.to_string(),
    };
    write_if_changed(&path, &patch_palworld(&base, port, &pw))?;
    Ok(pw)
}

fn apply_zomboid(dir: &Path, servername: &str, port: u16, pw: &str) -> std::io::Result<()> {
    if servername.is_empty() || servername.contains(['/', '\\']) || servername.contains("..") { return Ok(()); }
    let path = dir.join("Zomboid/Server").join(format!("{servername}.ini"));
    let Ok(old) = std::fs::read_to_string(&path) else { return Ok(()) }; // the game writes it on first start
    write_if_changed(&path, &set_properties(&old, &[("RCONPort", port.to_string()), ("RCONPassword", pw.into())]))
}

/// Everything a start needs for remote admin: returns the settings with `rcon_password` filled in.
pub fn prepare(dir: &Path, g: &Game, port: u16, cfg: &mut HashMap<String, String>) {
    let Ok(Some(pw)) = ensure_password(dir, g) else { return };
    let pw = if g.id == "palworld" { apply_palworld(dir, port, &pw).unwrap_or(pw) } else { pw };
    cfg.insert("rcon_password".into(), pw.clone());
    if g.id == "zomboid" { let _ = apply_zomboid(dir, cfg.get("servername").map_or("moss", String::as_str), port, &pw); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palworld_options_are_patched_in_place() {
        let t = "[/Script/Pal.PalGameWorldSettings]\nOptionSettings=(Difficulty=None,ServerDescription=\"a, (b)\",RCONEnabled=False,CrossplayPlatforms=(Steam,Xbox),AdminPassword=\"\")\n";
        let p = patch_palworld(t, 8211, "pw1");
        assert!(p.contains("RCONEnabled=True") && p.contains("RCONPort=8211") && p.contains("AdminPassword=\"pw1\""));
        assert!(p.contains("Difficulty=None,ServerDescription=\"a, (b)\"") && p.contains("CrossplayPlatforms=(Steam,Xbox)"));
        assert_eq!(p.matches("RCONEnabled").count(), 1);
        assert_eq!(patch_palworld(&p, 8211, "pw1"), p);
        assert!(p.starts_with("[/Script/Pal.PalGameWorldSettings]\nOptionSettings=(") && p.ends_with(")\n"));
        let fresh = patch_palworld("", 1, "x");
        assert_eq!(fresh, "[/Script/Pal.PalGameWorldSettings]\nOptionSettings=(RCONEnabled=True,RCONPort=1,AdminPassword=\"x\")\n");
    }

    fn scratch() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("moss-rcon-setup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn password_is_generated_once_and_kept() {
        let d = scratch();
        let g = super::super::get("gmod").unwrap();
        let a = ensure_password(&d, g).unwrap().unwrap();
        assert_eq!(a.len(), 20);
        assert_eq!(ensure_password(&d, g).unwrap().unwrap(), a);
        std::fs::write(d.join(SETTINGS_FILE), "password=abc").unwrap(); // no trailing newline
        let b = ensure_password(&d, g).unwrap().unwrap();
        let t = std::fs::read_to_string(d.join(SETTINGS_FILE)).unwrap();
        assert_eq!(t, format!("password=abc\nrcon_password={b}\n"));
        assert!(ensure_password(&d, super::super::get("terraria").unwrap()).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn palworld_and_zomboid_files_are_written_before_launch() {
        let d = scratch();
        std::fs::write(d.join("DefaultPalWorldSettings.ini"), "[/Script/Pal.PalGameWorldSettings]\nOptionSettings=(Difficulty=None)\n").unwrap();
        let mut cfg = read_settings(&d);
        prepare(&d, super::super::get("palworld").unwrap(), 8211, &mut cfg);
        let pw = cfg["rcon_password"].clone();
        let ini = std::fs::read_to_string(d.join("Pal/Saved/Config/LinuxServer/PalWorldSettings.ini")).unwrap();
        assert!(ini.contains("Difficulty=None") && ini.contains(&format!("AdminPassword=\"{pw}\"")));
        let z = super::super::get("zomboid").unwrap();
        let mut cfg = read_settings(&d);
        prepare(&d, z, 16261, &mut cfg); // no ini yet: nothing is created
        assert!(!d.join("Zomboid").exists());
        std::fs::create_dir_all(d.join("Zomboid/Server")).unwrap();
        std::fs::write(d.join("Zomboid/Server/moss.ini"), "PVP=true\nRCONPort=27015\n").unwrap();
        prepare(&d, z, 16261, &mut cfg);
        let ini = std::fs::read_to_string(d.join("Zomboid/Server/moss.ini")).unwrap();
        assert!(ini.contains("PVP=true\n") && ini.contains("RCONPort=16261\n") && ini.contains(&format!("RCONPassword={}\n", cfg["rcon_password"])));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_operator_set_palworld_admin_password_is_kept_and_used_for_rcon() {
        let dir = std::env::temp_dir().join(format!("moss-pal-own-{}", uuid::Uuid::new_v4()));
        let ini = dir.join("Pal/Saved/Config/LinuxServer/PalWorldSettings.ini");
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(&ini, "[/Script/Pal.PalGameWorldSettings]\nOptionSettings=(ServerName=\"x\",AdminPassword=\"mine123\")\n").unwrap();
        std::fs::write(dir.join(SETTINGS_FILE), "rcon_password=generated\n").unwrap();
        let mut cfg = HashMap::new();
        prepare(&dir, super::super::get("palworld").unwrap(), 8211, &mut cfg);
        assert_eq!(cfg["rcon_password"], "mine123");
        assert_eq!(read_settings(&dir)["rcon_password"], "mine123");
        let t = std::fs::read_to_string(&ini).unwrap();
        assert!(t.contains("AdminPassword=\"mine123\"") && t.contains("RCONEnabled=True"));
        assert_eq!(palworld_admin_password("OptionSettings=(AdminPassword=\"\")"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
