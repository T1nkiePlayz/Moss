-- Optional per-server resource limits (cgroup v2 on Linux). NULL = not set.
ALTER TABLE servers ADD COLUMN memory_limit_mb INTEGER;
ALTER TABLE servers ADD COLUMN cpu_limit_pct INTEGER;
ALTER TABLE servers ADD COLUMN pids_limit INTEGER;
