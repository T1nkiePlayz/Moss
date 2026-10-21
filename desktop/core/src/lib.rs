//! Everything the desktop app does that is not a window: find or start the Moss controller on a free loopback
//! port, remember it, wait until it answers, stop it, and decide which addresses the window may show.
pub mod health;
pub mod launch;
pub mod nav;
pub mod state;

pub use launch::{Handle, Paths, attach_or_start, attach_or_start_with, log_tail, stop, stop_pid};
pub use state::Controller;
