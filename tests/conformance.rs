//! Runs the lifecycle conformance table against the simulated runtime. A new runtime backend adds its own file next to
//! this one with a `Backend` implementation and one `run_suite` call.
mod suite;
use suite::{Backend, run_suite};
use serde_json::{Value, json};

struct Sim;
impl Backend for Sim {
    fn name(&self) -> &str { "sim" }
    fn create_body(&self) -> Value { json!({"runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512}) }
    fn crash_command(&self) -> Option<&str> { Some("crash") }
    fn echo_command(&self) -> (String, String) { ("say conformance-echo".into(), "conformance-echo".into()) }
}

#[tokio::test(flavor = "multi_thread")]
async fn simulated_runtime_meets_the_lifecycle_table() { run_suite(Sim, None).await; }
