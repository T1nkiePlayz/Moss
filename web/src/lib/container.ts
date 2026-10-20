// Container isolation wording and input checks (pure).
import type { ContainerStatus, ExtraPort } from './api';

export interface EngineInfo { tone: 'ok' | 'warn'; badge: string; detail: string }

/** What to tell the person about the container engine on this host. */
export function describeEngine(s: ContainerStatus | null | undefined): EngineInfo {
  if (!s) return { tone: 'warn', badge: 'Checking…', detail: '' };
  if (!s.available) return { tone: 'warn', badge: 'No container engine', detail: s.message ?? 'Neither Docker nor Podman was found. Install one of them, then check again.' };
  const name = s.engine === 'podman' ? 'Podman' : 'Docker';
  const mode = s.rootless ? 'rootless' : 'runs as root';
  const tip = s.engine === 'docker' && !s.rootless ? ' Rootless Podman keeps a container escape away from root: prefer it if you can.' : '';
  return { tone: 'ok', badge: `${name} ${s.version ?? ''} (${mode})`.replace('  ', ' '), detail: `Servers set to container mode run inside ${name}.${tip}` };
}

const REPOS = ['docker.io/library/eclipse-temurin', 'docker.io/azul/zulu-openjdk', 'docker.io/library/amazoncorretto'];
/** Quick client-side hint; the server validates strictly and is the authority. */
export function imageHint(raw: string): string | null {
  const v = raw.trim();
  if (v === '') return null;
  if (/\s/.test(v)) return 'An image name has no spaces.';
  if (!/[:@]/.test(v.replace(/^[^/]*\//, '')) && !/@sha256:/.test(v)) return 'Add a tag, for example :21-jre.';
  const full = v.startsWith('ghcr.io/') || REPOS.some((r) => v.startsWith(`${r}:`) || v.startsWith(`${r}@`)) || /^(eclipse-temurin|azul\/zulu-openjdk|amazoncorretto):/.test(v);
  return full ? null : 'Only eclipse-temurin, azul/zulu-openjdk, amazoncorretto and ghcr.io images are allowed.';
}

/** "19132/udp, 24454/tcp" (a bare number means TCP) to a port list, or the first problem in plain words. */
export function parseExtraPorts(text: string): { ports: ExtraPort[]; error: string | null } {
  const ports: ExtraPort[] = [];
  for (const raw of text.split(/[\s,]+/).filter(Boolean)) {
    const m = /^(\d{1,5})(?:\/(tcp|udp))?$/i.exec(raw);
    if (!m) return { ports, error: `"${raw}" is not a port. Write it like 19132/udp.` };
    const port = Number(m[1]);
    if (port < 1024 || port > 65535) return { ports, error: `${port} is outside 1024-65535.` };
    const proto = (m[2]?.toLowerCase() ?? 'tcp') as ExtraPort['proto'];
    if (ports.some((p) => p.port === port && p.proto === proto)) return { ports, error: `${port}/${proto} is listed twice.` };
    ports.push({ port, proto });
  }
  if (ports.length > 8) return { ports, error: 'At most 8 extra ports.' };
  return { ports, error: null };
}
export const formatExtraPorts = (p: ExtraPort[] | undefined) => (p ?? []).map((x) => `${x.port}/${x.proto}`).join(', ');

export const ISOLATION_NOTES = [
  'Only the server folder is shared with the container, the game port is the only port opened, and the container cannot gain extra privileges.',
  'With the Open network the server can still reach the internet and the rest of your network. Game-only blocks new outgoing connections to the internet.',
  'Game-only needs Docker. It also stops online-mode servers from reaching Mojang to check player accounts, so use it for servers behind a proxy that signs players in, or in offline mode. The computer Moss runs on stays reachable from the server.',
  'Plugins that need their own ports (voice chat, Geyser) work once you list those ports below.',
];
