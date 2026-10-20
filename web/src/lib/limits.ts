// Resource-limit explanations (pure).
import type { Limits } from './api';

export interface LimitInfo { badge: string; reason: string }
/** Why limits may not be enforced, tailored to the host OS. */
export function describeLimits(l: Limits | undefined, os?: string): LimitInfo {
  if (!l) return { badge: 'Unknown', reason: '' };
  if (l.enforced) return { badge: `Enforced (${l.backend === 'cgroup2' ? 'cgroup v2' : l.backend})`, reason: '' };
  const mac = /mac/i.test(os ?? '');
  return {
    badge: 'Not enforced',
    reason: mac
      ? 'macOS has no cgroups, so Moss cannot cap memory, CPU or processes on this host. The values are remembered and take effect if you move the instance to Linux.'
      : 'Moss needs a delegated cgroup v2 subtree to enforce limits. Run it under systemd with Delegate=yes (the shipped unit does), on a host that uses cgroup v2. In a container, mount the cgroup tree writable.',
  };
}
export function limitWarnings(o: { memory: string; memoryHeap: number }): string[] {
  const w: string[] = [];
  const m = o.memory.trim() === '' ? null : Number(o.memory);
  if (m !== null && isFinite(m) && m > 0 && m < o.memoryHeap) w.push(`The memory limit (${m} MB) is below the Java heap (${o.memoryHeap} MB). The server would be killed as soon as it fills its heap.`);
  return w;
}
