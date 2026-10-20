//! Curated catalogue of the commonly changed keys of each configuration file: type, default, description, limits
//! and the Minecraft versions they exist for. Anything not listed here is still editable as raw text and appears
//! under "Other settings" in the UI. Defaults are what a freshly generated file contains in recent releases.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty { Bool, Int, Float, Str }
impl Ty { pub fn name(self) -> &'static str { match self { Ty::Bool => "bool", Ty::Int => "int", Ty::Float => "float", Ty::Str => "string" } } }

#[derive(Clone, Copy, Debug)]
pub struct Key {
    /// Dotted path inside the file.
    pub path: &'static str,
    pub ty: Ty,
    /// The default as a JSON literal.
    pub default: &'static str,
    pub group: &'static str,
    pub desc: &'static str,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Allowed values (compared as text) for enumerations.
    pub options: &'static [&'static str],
    pub restart: bool,
    /// First version the key exists in, if later than the file itself.
    pub since: Option<&'static str>,
    pub until: Option<&'static str>,
    /// Never shown by the structured editor (the value is write-only).
    pub secret: bool,
    /// Changing it can break gameplay or security; the UI warns.
    pub danger: bool,
    /// A string setting that may also be a plain number ("default" or a distance).
    pub mixed: bool,
}

const fn k(path: &'static str, ty: Ty, default: &'static str, group: &'static str, desc: &'static str) -> Key {
    Key { path, ty, default, group, desc, min: None, max: None, options: &[], restart: false, since: None, until: None, secret: false, danger: false, mixed: false }
}
impl Key {
    const fn range(mut self, lo: f64, hi: f64) -> Self { self.min = Some(lo); self.max = Some(hi); self }
    const fn min(mut self, lo: f64) -> Self { self.min = Some(lo); self }
    const fn opts(mut self, o: &'static [&'static str]) -> Self { self.options = o; self }
    const fn restart(mut self) -> Self { self.restart = true; self }
    const fn since(mut self, v: &'static str) -> Self { self.since = Some(v); self }
    const fn secret(mut self) -> Self { self.secret = true; self }
    const fn danger(mut self) -> Self { self.danger = true; self }
    const fn mixed(mut self) -> Self { self.mixed = true; self }

    pub fn default_json(&self) -> Value { serde_json::from_str(self.default).unwrap_or(Value::Null) }

    /// Is the key present for this game (or proxy) version? An unparseable version counts as "yes".
    pub fn exists_in(&self, version: &str) -> bool {
        let v = parse_version(version);
        if v.is_empty() { return true; }
        self.since.is_none_or(|s| cmp_version(&v, &parse_version(s)).is_ge()) && self.until.is_none_or(|u| cmp_version(&v, &parse_version(u)).is_lt())
    }

    /// Check a proposed value against type, range and options.
    pub fn validate(&self, v: &Value) -> Result<(), String> {
        match (self.ty, v) {
            (Ty::Str, Value::String(s)) if self.mixed && s == "default" => return Ok(()),
            (Ty::Str, Value::Number(n)) if self.mixed && n.is_i64() => {}
            (Ty::Bool, Value::Bool(_)) => {}
            (Ty::Int, Value::Number(n)) if n.is_i64() => {}
            (Ty::Float, Value::Number(_)) => {}
            (Ty::Str, Value::String(s)) if s.chars().count() <= 2000 && !s.contains(['\n', '\r', '\0']) => {}
            (Ty::Str, Value::String(_)) => return Err("must be a single line of at most 2000 characters".into()),
            _ => return Err(format!("must be a {}", self.ty.name())),
        }
        if let Some(n) = v.as_f64() {
            if self.min.is_some_and(|m| n < m) || self.max.is_some_and(|m| n > m) {
                return Err(match (self.min, self.max) { (Some(a), Some(b)) => format!("must be between {a} and {b}"), (Some(a), None) => format!("must be at least {a}"), (None, Some(b)) => format!("must be at most {b}"), _ => unreachable!() });
            }
        }
        if !self.options.is_empty() {
            let t = match v { Value::String(s) => s.clone(), o => o.to_string() };
            if !self.options.contains(&t.as_str()) { return Err(format!("must be one of {}", self.options.join(", "))); }
        }
        Ok(())
    }
}

/// Leading numeric components of a version ("1.21.4", "3.4.0-SNAPSHOT" -> [3, 4, 0]).
pub fn parse_version(s: &str) -> Vec<u32> {
    let mut out = vec![];
    for part in s.split('.') {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(n) = digits.parse() else { break };
        out.push(n);
        if digits.len() != part.len() { break; }
    }
    out
}
pub fn cmp_version(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
    (0..a.len().max(b.len())).map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0))).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
}

use Ty::{Bool, Float, Int, Str};

pub static PAPER_GLOBAL: &[Key] = &[
    k("proxies.velocity.enabled", Bool, "false", "Proxy", "Accept players forwarded by a Velocity proxy. Turn on together with the secret below.").restart(),
    k("proxies.velocity.online-mode", Bool, "true", "Proxy", "Tell Paper that the Velocity proxy authenticates players with Mojang.").restart(),
    k("proxies.velocity.secret", Str, "\"\"", "Proxy", "The forwarding secret. It must be identical to the one in the proxy's forwarding.secret file.").restart().secret(),
    k("proxies.bungee-cord.online-mode", Bool, "true", "Proxy", "Tell Paper that a BungeeCord proxy authenticates players with Mojang.").restart(),
    k("proxies.proxy-protocol", Bool, "false", "Proxy", "Accept the HAProxy PROXY protocol. Only enable it when a load balancer in front of the server sends it.").restart().danger(),
    k("console.enable-brigadier-highlighting", Bool, "true", "Console", "Colour commands in the server console."),
    k("console.enable-brigadier-completions", Bool, "true", "Console", "Tab completion in the server console."),
    k("collisions.enable-player-collisions", Bool, "true", "Gameplay", "Players push each other. Turn off to make players pass through one another."),
    k("player-auto-save.rate", Int, "-1", "Saving", "How often (in ticks) each player's data is saved. -1 uses the world save interval.").min(-1.0),
    k("player-auto-save.max-per-tick", Int, "-1", "Saving", "At most this many players are saved per tick. -1 lets Paper decide.").min(-1.0),
    k("spam-limiter.tab-spam-increment", Int, "1", "Spam limits", "How much each tab completion request counts towards the spam limit.").min(0.0),
    k("spam-limiter.tab-spam-limit", Int, "500", "Spam limits", "Players are kicked for spam when tab completions add up to this much.").min(0.0),
    k("spam-limiter.recipe-spam-increment", Int, "1", "Spam limits", "How much each recipe book click counts towards the spam limit.").min(0.0),
    k("spam-limiter.recipe-spam-limit", Int, "20", "Spam limits", "Players are kicked for spam when recipe clicks add up to this much.").min(0.0),
    k("spam-limiter.incoming-packet-threshold", Int, "300", "Spam limits", "Packets per second above which a player's connection is treated as spam.").min(0.0),
    k("misc.max-joins-per-tick", Int, "3", "Performance", "How many players may finish joining in one tick. Lower it to smooth out a rush of logins.").range(1.0, 100.0),
    k("misc.use-alternative-luck-formula", Bool, "false", "Gameplay", "Use the alternative luck formula for loot tables."),
    k("misc.lag-compensate-block-breaking", Bool, "true", "Gameplay", "Make block breaking fair for players with high latency."),
    k("unsupported-settings.allow-permanent-block-break-exploits", Bool, "false", "Exploits", "Allow the permanent block breaking exploits (carpet and rail duplication). Not supported by Paper.").danger(),
    k("unsupported-settings.allow-piston-duplication", Bool, "false", "Exploits", "Allow the piston item duplication exploits. Not supported by Paper.").danger(),
    k("unsupported-settings.allow-headless-pistons", Bool, "false", "Exploits", "Allow headless pistons. Not supported by Paper.").danger(),
    k("messages.no-permission", Str, "\"<red>I'm sorry, but you do not have permission to perform this command. Please contact the server administrators if you believe that this is a mistake.\"", "Messages", "Shown when a player lacks permission for a command."),
];

pub static PAPER_WORLD: &[Key] = &[
    k("entities.spawning.spawn-limits.monsters", Int, "-1", "Spawning", "Monster cap per player area for this world. -1 uses the value in bukkit.yml.").min(-1.0),
    k("entities.spawning.spawn-limits.animals", Int, "-1", "Spawning", "Animal cap per player area. -1 uses the value in bukkit.yml.").min(-1.0),
    k("entities.spawning.spawn-limits.water-animals", Int, "-1", "Spawning", "Water animal (squid, dolphin) cap. -1 uses the value in bukkit.yml.").min(-1.0),
    k("entities.spawning.spawn-limits.water-ambient", Int, "-1", "Spawning", "Ambient water creature (fish) cap. -1 uses the value in bukkit.yml.").min(-1.0),
    k("entities.spawning.spawn-limits.ambient", Int, "-1", "Spawning", "Ambient creature (bat) cap. -1 uses the value in bukkit.yml.").min(-1.0),
    k("entities.spawning.per-player-mob-spawns", Bool, "true", "Spawning", "Share the mob cap fairly between players instead of across the whole world."),
    k("entities.spawning.all-chunks-are-slime-chunks", Bool, "false", "Spawning", "Slimes can spawn in every chunk, not just slime chunks."),
    k("entities.behavior.disable-chest-cat-detection", Bool, "false", "Mob behaviour", "Cats no longer stop chests from opening when sitting on them."),
    k("entities.behavior.nerf-pigmen-from-nether-portals", Bool, "false", "Mob behaviour", "Zombified piglins from nether portals have no AI. Cuts lag from portal farms."),
    k("entities.behavior.zombies-target-turtle-eggs", Bool, "true", "Mob behaviour", "Zombies trample turtle eggs."),
    k("chunks.max-auto-save-chunks-per-tick", Int, "24", "Chunks", "How many chunks are saved per tick during autosave.").range(1.0, 1000.0),
    k("chunks.prevent-moving-into-unloaded-chunks", Bool, "false", "Chunks", "Stop players from moving into chunks that are not loaded yet."),
    k("chunks.delay-chunk-unloads-by", Str, "\"10s\"", "Chunks", "How long a chunk stays loaded after the last player leaves it (for example 10s)."),
    k("environment.optimize-explosions", Bool, "false", "Environment", "Faster explosions, with slightly different results."),
    k("environment.disable-thunder", Bool, "false", "Environment", "No thunderstorms."),
    k("environment.disable-ice-and-snow", Bool, "false", "Environment", "Ice and snow never form."),
    k("environment.disable-explosion-knockback", Bool, "false", "Environment", "Explosions no longer push entities."),
    k("environment.treasure-maps.enabled", Bool, "true", "Environment", "Treasure maps work. Turn off to avoid lag from searching for structures."),
    k("environment.water-over-lava-flow-speed", Int, "5", "Environment", "How fast water flows over lava.").range(1.0, 100.0),
    k("anticheat.anti-xray.enabled", Bool, "false", "Anti-Xray", "Hide ores from X-ray texture packs and mods.").restart(),
    k("anticheat.anti-xray.engine-mode", Int, "1", "Anti-Xray", "1 hides ores, 2 fills the underground with fake ores (more effective, more CPU).").opts(&["1", "2"]).restart(),
    k("anticheat.anti-xray.max-block-height", Int, "64", "Anti-Xray", "Only blocks below this height are protected.").range(0.0, 320.0).restart(),
    k("anticheat.anti-xray.update-radius", Int, "2", "Anti-Xray", "How far around a changed block hidden blocks are revealed.").range(0.0, 2.0).restart(),
    k("anticheat.anti-xray.lava-obscures", Bool, "false", "Anti-Xray", "Lava hides ores next to it from view.").restart(),
    k("anticheat.anti-xray.use-permission", Bool, "false", "Anti-Xray", "Players with the paper.antixray.bypass permission see real ores.").restart(),
    k("maps.item-frame-cursor-limit", Int, "128", "Maps", "Most map cursors shown per map from item frames.").min(0.0),
    k("maps.item-frame-cursor-update-interval", Int, "10", "Maps", "How often (in ticks) item frame map cursors update.").min(1.0),
    k("hopper.disable-move-event", Bool, "false", "Hoppers", "Skip the item move event for hoppers. Faster, but plugins that listen to it stop working."),
    k("hopper.ignore-occluding-blocks", Bool, "false", "Hoppers", "Hoppers take items through solid blocks."),
    k("collisions.max-entity-collisions", Int, "8", "Performance", "Entities stop pushing each other after this many collisions. Lowering it limits lag from crowds.").min(0.0),
    k("collisions.fix-climbing-bypassing-cramming-rule", Bool, "false", "Performance", "Climbing entities count towards the cramming limit."),
    k("misc.disable-end-credits", Bool, "false", "Misc", "Skip the credits after beating the dragon."),
    k("misc.light-queue-size", Int, "20", "Misc", "Size of the lighting update queue.").min(1.0),
    k("misc.max-leash-distance", Float, "10.0", "Misc", "Distance at which a leash breaks.").min(1.0),
    k("misc.redstone-implementation", Str, "\"VANILLA\"", "Misc", "Redstone engine: VANILLA, EIGENCRAFT (faster, slightly different) or ALTERNATE_CURRENT (fastest, different timing).").opts(&["VANILLA", "EIGENCRAFT", "ALTERNATE_CURRENT"]),
];

pub static SPIGOT: &[Key] = &[
    k("settings.save-user-cache-on-stop-only", Bool, "false", "General", "Write usercache.json only when the server stops."),
    k("settings.moved-wrongly-threshold", Float, "0.0625", "Movement", "How far a player may differ from where the server expects them before being set back.").min(0.0),
    k("settings.moved-too-quickly-multiplier", Float, "10.0", "Movement", "Speed limit multiplier before a player is set back for moving too quickly.").min(0.0),
    k("settings.timeout-time", Int, "60", "General", "Seconds a server tick may take before the watchdog considers the server frozen.").min(1.0).restart(),
    k("settings.restart-on-crash", Bool, "true", "General", "Run the restart script after a crash. Moss has its own restart policy; leave this off for Moss-managed servers.").restart(),
    k("settings.netty-threads", Int, "4", "General", "Threads for network traffic.").range(1.0, 64.0).restart(),
    k("commands.log", Bool, "true", "Commands", "Write player commands to the server log."),
    k("commands.tab-complete", Int, "0", "Commands", "Tab completion: 0 always, -1 never, otherwise players need this many typed characters.").min(-1.0),
    k("commands.send-namespaced", Bool, "true", "Commands", "Show plugin:command style names in tab completion."),
    k("commands.silent-commandblock-console", Bool, "false", "Commands", "Do not print command block output to the console."),
    k("world-settings.default.view-distance", Str, "\"default\"", "World", "Chunks sent around each player. A number, or default to use server.properties.").mixed().range(2.0, 32.0),
    k("world-settings.default.simulation-distance", Str, "\"default\"", "World", "Chunks around each player that are simulated. A number, or default to use server.properties.").mixed().range(2.0, 32.0),
    k("world-settings.default.mob-spawn-range", Int, "8", "World", "How many chunks around a player mobs can spawn in.").range(1.0, 10.0),
    k("world-settings.default.item-despawn-rate", Int, "6000", "World", "Ticks until dropped items disappear (6000 is five minutes).").min(1.0),
    k("world-settings.default.arrow-despawn-rate", Int, "1200", "World", "Ticks until stuck arrows disappear.").min(1.0),
    k("world-settings.default.merge-radius.item", Float, "0.5", "World", "Dropped items within this many blocks merge into one stack.").min(0.0),
    k("world-settings.default.merge-radius.exp", Float, "-1.0", "World", "Experience orbs within this many blocks merge. -1 uses the default."),
    k("world-settings.default.nerf-spawner-mobs", Bool, "false", "World", "Mobs from spawners have no AI. Cuts lag from spawner farms."),
    k("world-settings.default.max-tick-time.tile", Int, "50", "World", "Milliseconds block entities may use per tick before the rest wait.").min(1.0),
    k("world-settings.default.max-tick-time.entity", Int, "50", "World", "Milliseconds entities may use per tick before the rest wait.").min(1.0),
    k("world-settings.default.entity-activation-range.animals", Int, "32", "Activation range", "Blocks from a player within which animals are fully active.").range(0.0, 256.0),
    k("world-settings.default.entity-activation-range.monsters", Int, "32", "Activation range", "Blocks from a player within which monsters are fully active.").range(0.0, 256.0),
    k("world-settings.default.entity-activation-range.raiders", Int, "48", "Activation range", "Blocks from a player within which raiders are fully active.").range(0.0, 256.0),
    k("world-settings.default.entity-activation-range.misc", Int, "16", "Activation range", "Blocks from a player within which other entities are fully active.").range(0.0, 256.0),
    k("world-settings.default.entity-tracking-range.players", Int, "48", "Tracking range", "Blocks within which other players are shown.").range(8.0, 512.0),
    k("world-settings.default.entity-tracking-range.animals", Int, "48", "Tracking range", "Blocks within which animals are shown.").range(8.0, 512.0),
    k("world-settings.default.entity-tracking-range.monsters", Int, "48", "Tracking range", "Blocks within which monsters are shown.").range(8.0, 512.0),
];

pub static BUKKIT: &[Key] = &[
    k("settings.allow-end", Bool, "true", "General", "Allow travel to the End.").restart(),
    k("settings.warn-on-overload", Bool, "true", "General", "Log a warning when the server falls behind."),
    k("settings.connection-throttle", Int, "4000", "General", "Milliseconds a client must wait before reconnecting. -1 disables the limit.").min(-1.0).restart(),
    k("settings.plugin-profiling", Bool, "false", "General", "Collect timing data for plugins."),
    k("settings.shutdown-message", Str, "\"Server closed\"", "General", "Message shown to players when the server stops."),
    k("settings.query-plugins", Bool, "true", "General", "List plugins in query responses."),
    k("spawn-limits.monsters", Int, "70", "Spawn limits", "Most monsters that can spawn around each player.").min(0.0),
    k("spawn-limits.animals", Int, "10", "Spawn limits", "Most animals that can spawn around each player.").min(0.0),
    k("spawn-limits.water-animals", Int, "5", "Spawn limits", "Most water animals (squid, dolphins) around each player.").min(0.0),
    k("spawn-limits.water-ambient", Int, "20", "Spawn limits", "Most ambient water creatures (fish) around each player.").min(0.0),
    k("spawn-limits.water-underground-creature", Int, "5", "Spawn limits", "Most glow squid around each player.").min(0.0).since("1.17"),
    k("spawn-limits.axolotls", Int, "5", "Spawn limits", "Most axolotls around each player.").min(0.0).since("1.17"),
    k("spawn-limits.ambient", Int, "15", "Spawn limits", "Most ambient creatures (bats) around each player.").min(0.0),
    k("chunk-gc.period-in-ticks", Int, "600", "Chunks", "How often unused chunks are unloaded, in ticks.").min(1.0),
    k("ticks-per.animal-spawns", Int, "400", "Spawn timing", "Ticks between animal spawn attempts. Higher is less laggy and slower.").min(1.0),
    k("ticks-per.monster-spawns", Int, "1", "Spawn timing", "Ticks between monster spawn attempts. Higher is less laggy and slower.").min(1.0),
    k("ticks-per.water-spawns", Int, "1", "Spawn timing", "Ticks between water animal spawn attempts.").min(1.0),
    k("ticks-per.water-ambient-spawns", Int, "1", "Spawn timing", "Ticks between ambient water creature spawn attempts.").min(1.0),
    k("ticks-per.ambient-spawns", Int, "1", "Spawn timing", "Ticks between ambient creature spawn attempts.").min(1.0),
    k("ticks-per.autosave", Int, "6000", "Saving", "Ticks between world autosaves (6000 is five minutes).").min(0.0),
];

pub static PURPUR: &[Key] = &[
    k("settings.server-mod-name", Str, "\"Purpur\"", "General", "Name reported to players and in server lists."),
    k("settings.use-alternate-keepalive", Bool, "false", "Network", "Use Purpur's keep-alive handling, which tolerates short lag spikes better. Fixes players timing out on bad connections."),
    k("settings.network.upnp-port-forwarding", Bool, "false", "Network", "Ask the router to open the server port automatically.").restart(),
    k("world-settings.default.mobs.villager.lobotomize.enabled", Bool, "false", "Mobs", "Villagers that are stuck think less often. Reduces lag from villager halls."),
];

pub static VELOCITY: &[Key] = &[
    k("bind", Str, "\"0.0.0.0:25577\"", "General", "Address and port the proxy listens on.").restart(),
    k("motd", Str, "\"<#09add3>A Velocity Server\"", "General", "The message in the server list (MiniMessage format)."),
    k("show-max-players", Int, "500", "General", "The maximum number shown in the server list. It is not a hard limit.").min(0.0),
    k("online-mode", Bool, "true", "Security", "Check player accounts with Mojang. Turning it off lets anyone join under any name.").restart().danger(),
    k("force-key-authentication", Bool, "true", "Security", "Require the signed key that Minecraft 1.19 and newer clients send.").restart(),
    k("player-info-forwarding-mode", Str, "\"NONE\"", "Security", "How player identity is passed to backend servers. Use MODERN for current Paper servers, with the secret set on both sides.").opts(&["NONE", "LEGACY", "BUNGEEGUARD", "MODERN"]).restart(),
    k("prevent-client-proxy-connections", Bool, "false", "Security", "Reject players who connect through a VPN or proxy different from the one used for authentication.").restart(),
    k("announce-forge", Bool, "false", "General", "Let Forge clients see that this proxy supports Forge servers.").restart(),
    k("kick-existing-players", Bool, "false", "General", "A second login with the same account kicks the first one."),
    k("ping-passthrough", Str, "\"DISABLED\"", "General", "Show the backend server's ping response (MOTD, player count) instead of the proxy's.").opts(&["DISABLED", "MODS", "DESCRIPTION", "ALL"]),
    k("advanced.compression-threshold", Int, "256", "Advanced", "Packets larger than this many bytes are compressed. -1 turns compression off.").min(-1.0).restart(),
    k("advanced.compression-level", Int, "-1", "Advanced", "Compression strength, -1 for the default.").range(-1.0, 9.0).restart(),
    k("advanced.login-ratelimit", Int, "3000", "Advanced", "Milliseconds between logins from the same address. 0 disables the limit.").min(0.0).restart(),
    k("advanced.connection-timeout", Int, "5000", "Advanced", "Milliseconds to wait when connecting to a backend server.").min(100.0).restart(),
    k("advanced.read-timeout", Int, "30000", "Advanced", "Milliseconds without data before a connection is dropped.").min(1000.0).restart(),
    k("advanced.haproxy-protocol", Bool, "false", "Advanced", "Accept the HAProxy PROXY protocol. Only enable it behind a load balancer that sends it.").restart().danger(),
    k("advanced.tcp-fast-open", Bool, "false", "Advanced", "Enable TCP Fast Open (Linux).").restart(),
    k("advanced.show-ping-requests", Bool, "false", "Advanced", "Log every server list ping."),
    k("advanced.failover-on-unexpected-server-disconnect", Bool, "true", "Advanced", "Send players to the next server in the try list when a backend closes unexpectedly."),
    k("advanced.log-command-executions", Bool, "false", "Advanced", "Write every command players run through the proxy to the log."),
    k("advanced.log-player-connections", Bool, "true", "Advanced", "Log players connecting and disconnecting."),
    k("query.enabled", Bool, "false", "Query", "Answer GameSpy query requests.").restart(),
    k("query.port", Int, "25577", "Query", "UDP port for query requests.").range(1.0, 65535.0).restart(),
    k("query.show-plugins", Bool, "false", "Query", "List installed proxy plugins in query responses."),
];

#[cfg(test)]
mod tests {
    use super::*;
    const ALL: &[(&str, &[Key])] = &[("paper-global", PAPER_GLOBAL), ("paper-world", PAPER_WORLD), ("spigot", SPIGOT), ("bukkit", BUKKIT), ("purpur", PURPUR), ("velocity", VELOCITY)];

    #[test]
    fn every_default_is_valid_for_its_own_key() {
        for (file, keys) in ALL {
            let mut seen = std::collections::HashSet::new();
            for key in *keys {
                assert!(seen.insert(key.path), "{file}: duplicate {}", key.path);
                let d = key.default_json();
                assert!(!d.is_null(), "{file}: {} default does not parse", key.path);
                if key.secret { continue; }
                key.validate(&d).unwrap_or_else(|e| panic!("{file}: {} default {d}: {e}", key.path));
                assert!(key.min.zip(key.max).is_none_or(|(a, b)| a <= b));
                assert!(!key.desc.is_empty() && !key.group.is_empty());
                // a key may not be both a value and the parent of another key
                assert!(!keys.iter().any(|o| o.path.starts_with(&format!("{}.", key.path))), "{file}: {} is also a section", key.path);
            }
        }
    }
    #[test]
    fn version_windows() {
        let bukkit_axolotl = BUKKIT.iter().find(|k| k.path == "spawn-limits.axolotls").unwrap();
        assert!(bukkit_axolotl.exists_in("1.17") && bukkit_axolotl.exists_in("1.21.4") && !bukkit_axolotl.exists_in("1.16.5"));
        assert!(bukkit_axolotl.exists_in("weird"), "unparseable versions are not hidden");
        assert_eq!(parse_version("3.4.0-SNAPSHOT"), [3, 4, 0]);
        assert_eq!(parse_version("1.21.4"), [1, 21, 4]);
        assert!(cmp_version(&[1, 21], &[1, 21, 0]).is_eq() && cmp_version(&[1, 9], &[1, 10]).is_lt());
    }
    #[test]
    fn validation_rules() {
        let k = BUKKIT.iter().find(|k| k.path == "spawn-limits.monsters").unwrap();
        assert!(k.validate(&serde_json::json!(70)).is_ok());
        assert!(k.validate(&serde_json::json!(-1)).is_err());
        assert!(k.validate(&serde_json::json!("70")).is_err());
        assert!(k.validate(&serde_json::json!(1.5)).is_err());
        let o = PAPER_WORLD.iter().find(|k| k.path == "anticheat.anti-xray.engine-mode").unwrap();
        assert!(o.validate(&serde_json::json!(2)).is_ok() && o.validate(&serde_json::json!(3)).is_err());
        let s = SPIGOT.iter().find(|k| k.path == "settings.timeout-time").unwrap();
        assert!(s.validate(&serde_json::json!("a\nb")).is_err());
    }
}
