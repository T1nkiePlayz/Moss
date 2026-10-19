// server.properties helpers: describe the friendly fields and patch raw text in place.
export type PropType = 'bool' | 'number' | 'select' | 'text';
export interface PropDef {
  key: string;
  label: string;
  type: PropType;
  options?: string[];
  min?: number;
  max?: number;
  hint?: string;
  restart?: boolean;
}
export interface PropGroup { title: string; items: PropDef[] }

export const PROP_GROUPS: PropGroup[] = [
  {
    title: 'General',
    items: [
      { key: 'motd', label: 'Message of the day', type: 'text' },
      { key: 'max-players', label: 'Max players', type: 'number', min: 0, max: 100000 },
      { key: 'gamemode', label: 'Default game mode', type: 'select', options: ['survival', 'creative', 'adventure', 'spectator'] },
      { key: 'difficulty', label: 'Difficulty', type: 'select', options: ['peaceful', 'easy', 'normal', 'hard'] },
      { key: 'hardcore', label: 'Hardcore', type: 'bool', restart: true },
      { key: 'pvp', label: 'PvP', type: 'bool' },
    ],
  },
  {
    title: 'World',
    items: [
      { key: 'level-name', label: 'World folder', type: 'text', restart: true },
      { key: 'level-seed', label: 'Seed', type: 'text', restart: true },
      { key: 'level-type', label: 'World type', type: 'text', restart: true },
      { key: 'view-distance', label: 'View distance', type: 'number', min: 2, max: 32 },
      { key: 'simulation-distance', label: 'Simulation distance', type: 'number', min: 2, max: 32 },
      { key: 'spawn-protection', label: 'Spawn protection', type: 'number', min: 0, max: 1000 },
      { key: 'allow-nether', label: 'Nether', type: 'bool', restart: true },
      { key: 'generate-structures', label: 'Generate structures', type: 'bool', restart: true },
      { key: 'spawn-monsters', label: 'Spawn monsters', type: 'bool' },
    ],
  },
  {
    title: 'Access & network',
    items: [
      { key: 'online-mode', label: 'Online mode (verify accounts)', type: 'bool', restart: true },
      { key: 'white-list', label: 'Whitelist', type: 'bool' },
      { key: 'enforce-whitelist', label: 'Enforce whitelist', type: 'bool' },
      { key: 'server-port', label: 'Server port', type: 'number', min: 1, max: 65535, restart: true, hint: 'Prefer changing the port in Settings so Moss stays in sync.' },
      { key: 'enable-command-block', label: 'Command blocks', type: 'bool' },
      { key: 'allow-flight', label: 'Allow flight', type: 'bool' },
    ],
  },
];

export interface Entry { key: string; value: string }

export function parseProps(raw: string): Entry[] {
  const out: Entry[] = [];
  for (const line of raw.split('\n')) {
    const l = line.trim();
    if (!l || l.startsWith('#')) continue;
    const i = l.indexOf('=');
    if (i > 0) out.push({ key: l.slice(0, i).trim(), value: l.slice(i + 1) });
  }
  return out;
}

/** Return `raw` with key set to value, preserving comments and order; appends if missing. */
export function setProp(raw: string, key: string, value: string): string {
  const lines = raw.split('\n');
  let found = false;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    if (l.trimStart().startsWith('#')) continue;
    const idx = l.indexOf('=');
    if (idx > 0 && l.slice(0, idx).trim() === key) {
      lines[i] = `${key}=${value}`;
      found = true;
    }
  }
  if (!found) {
    if (lines.length && lines[lines.length - 1] === '') lines.splice(lines.length - 1, 0, `${key}=${value}`);
    else lines.push(`${key}=${value}`);
  }
  return lines.join('\n');
}
