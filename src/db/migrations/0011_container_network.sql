-- Container network mode ('open' | 'game-only') and extra published ports (JSON [{"port":19132,"proto":"udp"}]).
ALTER TABLE servers ADD COLUMN network TEXT NOT NULL DEFAULT 'open';
ALTER TABLE servers ADD COLUMN extra_ports TEXT NOT NULL DEFAULT '[]';
