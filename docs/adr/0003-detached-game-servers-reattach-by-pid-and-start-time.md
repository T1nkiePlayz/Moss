# 0003. Game servers are detached and re-attached by PID and start time

Status: Accepted

## Context

Restarting or upgrading the controller must not kick players or lose a world. If servers were children of Moss, a Moss crash, upgrade or `systemctl restart` would stop them.

## Decision

`src/lifecycle/start.rs` launches each server through `/bin/sh` with `setsid`. A wrapper writes its PID, runs the game with stdin bound to a FIFO the wrapper holds open read-write (so the game never sees EOF), appends output to `console.log`, and writes the exit code to a file. Moss stores the PID and the process start time. On boot `reconcile` (`src/lifecycle/reconcile.rs`) checks each installed server: if the identity still matches it re-attaches (restarts the log tailer, restores state); if not, it accounts for the exit (clean stop or crash, restart policy) and autostarts where configured. The systemd unit uses `KillMode=process`.

## Consequences

- Servers survive controller crashes (`SIGKILL`), upgrades and restarts; an end-to-end test exercises this.
- PID alone is unsafe because PIDs are reused; the start time (read from `/proc` on Linux, libproc on macOS, via `src/platform/`) makes the identity check reliable.
- Console input goes through a FIFO and output through a file, so console history also survives restarts.
- Exit information is inferred from files and process state, not from `wait()`, so edge cases (machine reboot, killed wrapper) need explicit handling.
- Servers run as the Moss user and are not contained; detachment is about lifecycle, not isolation.
- Cgroup limits are applied by the launch script moving the process group into a leaf (`src/platform/cgroup.rs`), and cleaned up when the server is gone.
