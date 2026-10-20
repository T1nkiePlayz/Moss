//! Moss entry point: module tree, CLI definition and command dispatch. Each command lives in its own module.
mod api; mod auth; mod backups; mod config; mod console; mod container; mod content; mod db; mod doctor; mod dotenv; mod download; mod error; mod files;
mod health; mod history; mod installer; mod jars; mod java; mod lifecycle; mod mcping; mod notify; mod ops; mod platform; mod providers; mod runtimes; mod sched; mod serve; mod server; mod sim; mod state; mod undo;
#[cfg(test)] mod testutil;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "moss", version, about = "Moss — a fast, local-first game server controller")]
struct Cli { #[command(subcommand)] cmd: Cmd }

#[derive(Subcommand)]
enum Cmd {
    /// Run the controller (web panel + API)
    Serve(config::Config),
    /// Check the environment: data directory, ports, Java, auth configuration
    Doctor(config::Config),
    /// A tiny fake Minecraft server, used by the `sim` runtime for demos and tests
    SimServer { #[arg(long, default_value_t = 25565)] port: u16 },
}

fn main() -> anyhow::Result<()> {
    dotenv::load();
    let cli = Cli::parse();
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "moss=info,tower_http=warn".into())).with_target(false).init();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    match cli.cmd {
        Cmd::Serve(cfg) => rt.block_on(serve::serve(cfg)),
        Cmd::Doctor(cfg) => rt.block_on(doctor::doctor(cfg)),
        Cmd::SimServer { port } => rt.block_on(sim::run(port)),
    }
}
