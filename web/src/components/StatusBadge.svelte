<script lang="ts">
  import type { ServerState } from '../lib/api';
  let { state, detail }: { state: ServerState; detail?: string | null } = $props();
  const L: Record<ServerState, string> = {
    installing: 'Installing', stopped: 'Stopped', starting: 'Starting', running: 'Running', stopping: 'Stopping',
    crashed: 'Crashed', failed: 'Failed', needs_attention: 'Needs attention',
  };
  const tone = $derived(state === 'running' ? 'ok' : state === 'crashed' || state === 'failed' ? 'danger' : state === 'stopped' ? '' : 'warn');
</script>

<span class="badge {tone}" title={detail ?? undefined}><span class="dot {state}"></span>{L[state]}</span>
