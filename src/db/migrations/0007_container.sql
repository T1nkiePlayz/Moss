-- Optional container isolation. isolation: 'process' (default) | 'container'. container_image: allow-listed override.
-- in_container records how the CURRENT run was launched (a mode change only applies on the next start).
ALTER TABLE servers ADD COLUMN isolation TEXT NOT NULL DEFAULT 'process';
ALTER TABLE servers ADD COLUMN container_image TEXT;
ALTER TABLE servers ADD COLUMN in_container INTEGER NOT NULL DEFAULT 0;
