//! The fake Minecraft server behind the `sim-server` command (used by the `sim` runtime for demos and tests).
use crate::mcping;

/// Fake Minecraft server: prints realistic logs, answers status pings, handles a few console commands.
pub async fn run(port: u16) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt;
    let t = chrono::Local::now;
    let log = |lvl: &str, m: &str| println!("[{}] [Server thread/{lvl}]: {m}", t().format("%H:%M:%S"));
    log("INFO", "Starting minecraft server version sim-1.0");
    log("INFO", &format!("Starting Minecraft server on *:{port}"));
    tokio::spawn(mcping::serve_sim(port));
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    log("INFO", "Preparing level \"world\"");
    log("WARN", "Can't keep up! Is the server overloaded? (simulated)");
    log("INFO", "Done (1.204s)! For help, type \"help\"");
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
