<script lang="ts">
  import States from '../../components/States.svelte';
  import { apiExtra } from '../../lib/api-extra';
  import { errMsg, type Server, type ServerMap } from '../../lib/api';

  let { server: s }: { server: Server } = $props();
  let info = $state<ServerMap | null>(null);
  let err = $state('');
  let seq = 0;
  const names = { bluemap: 'BlueMap', squaremap: 'squaremap', pl3xmap: 'Pl3xMap', dynmap: 'dynmap' } as const;

  async function load() {
    const my = ++seq;
    err = '';
    try { const r = await apiExtra.map(s.id); if (my === seq) info = r; } catch (e) { if (my === seq) err = errMsg(e); }
  }
  // Reload when the server changes, starts or stops; the link is signed and expires, so renew it well before then.
  $effect(() => {
    s.id; s.state; info = null;
    void load();
    const t = setInterval(load, 6 * 3600 * 1000);
    return () => clearInterval(t);
  });
  const src = $derived(info?.available && info.reachable ? info.url : undefined);
</script>

{#if err}
  <States kind="error" title="Couldn't load the map" text={err} retry={load} />
{:else if !info}
  <States kind="loading" rows={2} />
{:else if !info.available}
  <States kind="empty" icon="layers" title="No map plugin found" text="Install BlueMap, squaremap, Pl3xMap or dynmap and enable its built-in web server." />
{:else if !src}
  <States kind="empty" icon="layers" title={info.running ? 'The map is not answering yet' : 'The server is stopped'} text={info.running ? `${names[info.kind!]} may still be rendering or starting its web server.` : 'Start the server to see the live map.'}>
    <button class="btn" onclick={load}>Check again</button>
  </States>
{:else}
  <div class="col g8">
    <div class="row g8 wrap"><span class="muted">{names[info.kind!]} live map</span><a class="btn" href={src} target="_blank" rel="noopener noreferrer">Open in a new tab</a></div>
    <!-- sandbox without allow-same-origin: the map is plugin-controlled code and must not run as Moss -->
    <iframe class="map" title="{s.name} live map" {src} sandbox="allow-scripts allow-popups" referrerpolicy="no-referrer"></iframe>
  </div>
{/if}

<style>
  .map { width: 100%; height: 70vh; min-height: 360px; border: 1px solid var(--line, #888); border-radius: 12px; background: var(--card, #fff); }
</style>
