// Container isolation wording and input checks (pure).
import type { ContainerStatus } from './api';

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

export const ISOLATION_NOTES = [
  'Only the server folder is shared with the container, the game port is the only port opened, and the container cannot gain extra privileges.',
  'The server can still reach the internet and the rest of your network: outbound traffic is not restricted.',
  'Plugin ports other than the game port (Geyser, voice chat) are not opened.',
];
