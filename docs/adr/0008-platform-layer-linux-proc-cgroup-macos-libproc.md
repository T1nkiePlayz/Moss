# 0008. A platform layer: Linux /proc and cgroup v2, macOS libproc

Status: Accepted

## Context

Moss needs process identity (PID plus start time), per-process CPU and memory, host statistics, disk space and Java discovery on Linux and macOS. Scattering `#[cfg]` and `/proc` reads through the code would make behaviour differ silently and make tests depend on the host.

## Decision

All OS-specific code lives in `src/platform/`: `mod.rs` is the interface, `linux.rs` reads `/proc`, `macos.rs` uses libproc, sysctl and a few tiny subprocesses (`vm_stat`, `ps`). Nothing else in Moss reads `/proc`. On Linux, `cgroup.rs` implements optional resource limits with cgroup v2: it needs a delegated subtree (the shipped unit sets `Delegate=yes` and `DelegateSubgroup=supervisor`), creates `moss-servers/` with one leaf per server, and writes `memory.max`, `memory.swap.max`, `cpu.max` and `pids.max`; OOM kills are read from the kernel counters. The cgroup mount point is a parameter so tests use a fake tree. When no backend is available, limits are reported as not enforced instead of failing.

## Consequences

- Process identity and statistics have one implementation per OS, and the re-attach logic in ADR 0003 depends only on that interface.
- macOS has no enforceable limits; the API and UI say so.
- Linux limits need systemd delegation (or an equivalent); without it Moss runs fine but unlimited.
- cgroups bound resource use, not access. They are not a security boundary and do not replace container isolation (see the threat model).
- Other platforms are unsupported until someone writes a third backend.
