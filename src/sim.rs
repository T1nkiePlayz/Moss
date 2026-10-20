//! The fake Minecraft server behind the `sim-server` command (used by the `sim` runtime for demos and tests).
use crate::mcping;

/// Optional `sim-behavior.txt` in the server directory (one setting per line) so tests can produce unhealthy servers:
/// `never-ready`, `fatal`, `crash-after-ready=N` (seconds), `error-lines=N`, `warn-lines=N`.
#[derive(Default)]
struct Behavior { never_ready: bool, fatal: bool, crash_after: Option<u64>, error_lines: u32, warn_lines: u32 }

impl Behavior {
    fn read() -> Self { Self::parse(&std::fs::read_to_string("sim-behavior.txt").unwrap_or_default()) }
    fn parse(text: &str) -> Self {
        let mut b = Behavior::default();
        for l in text.lines().map(str::trim) {
            let (k, v) = l.split_once('=').unwrap_or((l, ""));
            match k.trim() {
                "never-ready" => b.never_ready = true,
                "fatal" => b.fatal = true,
                "crash-after-ready" => b.crash_after = v.trim().parse().ok(),
                "error-lines" => b.error_lines = v.trim().parse().unwrap_or(0).min(1000),
                "warn-lines" => b.warn_lines = v.trim().parse().unwrap_or(0).min(1000),
                _ => {}
            }
        }
        b
    }
}

/// Fake Minecraft server: prints realistic logs, answers status pings, handles a few console commands.
pub async fn run(port: u16) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt;
    let t = chrono::Local::now;
    let log = |lvl: &str, m: &str| println!("[{}] [Server thread/{lvl}]: {m}", t().format("%H:%M:%S"));
    let b = Behavior::read();
    log("INFO", "Starting minecraft server version sim-1.0");
    log("INFO", &format!("Starting Minecraft server on *:{port}"));
    if !b.never_ready { tokio::spawn(mcping::serve_sim(port)); }
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    log("INFO", "Preparing level \"world\"");
    log("WARN", "Can't keep up! Is the server overloaded? (simulated)");
    for i in 0..b.warn_lines { log("WARN", &format!("Simulated warning {i}")); }
    for i in 0..b.error_lines { log("ERROR", &format!("Simulated error {i}")); }
    if !b.never_ready { log("INFO", "Done (1.204s)! For help, type \"help\""); }
    if b.fatal { log("ERROR", "java.lang.NoClassDefFoundError: org/example/Missing (simulated)"); }
    if let Some(n) = b.crash_after {
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(n)).await;
            println!("[{}] [Server thread/ERROR]: Simulated crash: java.lang.OutOfMemoryError: Java heap space", chrono::Local::now().format("%H:%M:%S"));
            std::process::exit(1);
        });
    }
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(l)) = lines.next_line().await {
        match l.trim() {
            "stop" => { log("INFO", "Stopping the server"); log("INFO", "Saving worlds"); tokio::time::sleep(std::time::Duration::from_millis(400)).await; return Ok(()); }
            "crash" => { log("ERROR", "Simulated crash: java.lang.OutOfMemoryError: Java heap space"); std::process::exit(1); }
            "save-off" => log("INFO", "Automatic saving is now disabled"),
            "save-on" => log("INFO", "Automatic saving is now enabled"),
            "save-all flush" => log("INFO", "Saved the game"),
            "list" => log("INFO", "There are 2 of a max of 20 players online: Steve, Alex"),
            c if c.starts_with("say ") => log("INFO", &format!("[Server] {}", &c[4..])),
            "" => {}
            c => log("INFO", &format!("Unknown or incomplete command: {c}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavior_file_parses_known_settings_and_ignores_the_rest() {
        let b = Behavior::parse("never-ready\nfatal\ncrash-after-ready = 3\nerror-lines=5\nwarn-lines=2000\nbogus=1\n");
        assert!(b.never_ready && b.fatal);
        assert_eq!((b.crash_after, b.error_lines, b.warn_lines), (Some(3), 5, 1000));
        let d = Behavior::parse("");
        assert!(!d.never_ready && !d.fatal && d.crash_after.is_none());
    }
}
