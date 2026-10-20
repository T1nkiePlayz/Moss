<script lang="ts">
  import Icon from './Icon.svelte';
  import { apiOps, ApiError, type ContainerStatus, type Server } from '../lib/api';
  import { toast, toastError } from '../lib/stores.svelte';
  import { can, isAdmin } from '../lib/auth.svelte';
  import { describeEngine, imageHint, ISOLATION_NOTES } from '../lib/container';

  let { server: s }: { server: Server } = $props();
  const canEdit = $derived(can('config.write') && isAdmin());
  const seedOf = () => ({ mode: s.isolation ?? 'process', image: s.container?.image_override ?? '' });
  let f = $state(seedOf());
  let seeded = '';
  $effect(() => { const k = s.id + JSON.stringify([s.isolation, s.container?.image_override]); if (k !== seeded) { seeded = k; f = seedOf(); } });
  let eng = $state<ContainerStatus | null>(null);
  let checking = $state(false);
  let saving = $state(false);
  let pulling = $state(false);
  let errs = $state<Record<string, string>>({});
  const dirty = $derived(JSON.stringify(f) !== JSON.stringify(seedOf()));
  const info = $derived(describeEngine(eng));
  const hint = $derived(imageHint(f.image));
  const unsupported = $derived(s.runtime === 'custom');
  const effective = $derived(f.image.trim() || s.container?.image || `eclipse-temurin:${s.java_major}-jre`);

  async function check() {
    checking = true;
    try { eng = await apiOps.containerStatus(); } catch (er) { toastError(er, 'Could not check for Docker or Podman'); }
    checking = false;
  }
  $effect(() => { check(); });

  async function save(e: Event) {
    e.preventDefault(); saving = true; errs = {};
    try {
      await apiOps.patchIsolation(s.id, { isolation: f.mode, container_image: f.image.trim() === '' ? null : f.image.trim() });
      const running = s.state === 'running' || s.state === 'starting';
      toast('success', 'Isolation saved', { body: running ? 'Restart the server to switch.' : f.mode === 'container' ? 'Moss is downloading the image now.' : undefined });
    } catch (er) {
      if (er instanceof ApiError) errs = Object.fromEntries(er.fields.map((x) => [x.path, x.message]));
      toastError(er, 'Could not save isolation');
    }
    saving = false;
  }

  async function pull() {
    pulling = true;
    try { await apiOps.pullImage(s.id); toast('info', 'Downloading the image', { body: 'Progress is shown under Operations.' }); } catch (er) { toastError(er, 'Could not start the download'); }
    pulling = false;
  }
</script>

<section class="card" aria-labelledby="iso-t">
  <div class="card-title" id="iso-t"><Icon name="shield" size={14} />Isolation</div>
  <div class="status" aria-live="polite">
    <span class="badge {info.tone}">{#if info.tone === 'ok'}<Icon name="check" size={12} />{/if}{info.badge}</span>
    <button class="btn sm" type="button" onclick={check} disabled={checking}>{checking ? 'Checking…' : 'Check again'}</button>
  </div>
  {#if info.detail}<p class="muted sm" style="margin:8px 0 0">{info.detail}</p>{/if}
  {#if eng && !eng.available && (s.isolation === 'container')}
    <div class="banner warn" style="margin-top:12px"><Icon name="alert" size={16} />This server is set to run in a container, but no container engine is available, so it cannot start until one is installed or you switch back to the normal mode.</div>
  {/if}
  {#if s.container?.restart_needed}
    <div class="banner warn" style="margin-top:12px"><Icon name="refresh" size={16} />The running server still uses {s.container.running_in_container ? 'a container' : 'the normal mode'}. Restart it to apply the saved isolation mode.</div>
  {/if}

  <form class="lf" onsubmit={save}>
    <div class="field">
      <label for="iso-m">Mode</label>
      <select id="iso-m" class="select" bind:value={f.mode} disabled={!canEdit || unsupported} aria-describedby="iso-mh">
        <option value="process">Normal (runs as the Moss user)</option>
        <option value="container" disabled={!eng?.available && f.mode !== 'container'}>Container (Docker or Podman)</option>
      </select>
      <span class="hint" id="iso-mh">{unsupported ? 'Custom servers cannot run in a container.' : 'A change applies the next time the server starts. Only owners and admins can change it.'}</span>
      {#if errs.isolation}<span class="err">{errs.isolation}</span>{/if}
    </div>
    <div class="field">
      <label for="iso-i">Image</label>
      <input id="iso-i" class="input" bind:value={f.image} placeholder="Default ({effective})" disabled={!canEdit || f.mode !== 'container'} spellcheck="false" autocomplete="off" aria-describedby="iso-ih" />
      <span class="hint" id="iso-ih">Leave empty to use the Java {s.java_major} image from Eclipse Temurin. Allowed: eclipse-temurin, azul/zulu-openjdk, amazoncorretto and ghcr.io images.</span>
      {#if hint}<span class="err">{hint}</span>{/if}
      {#if errs.container_image}<span class="err">{errs.container_image}</span>{/if}
    </div>
    {#if f.mode === 'container'}
      <ul class="notes full">{#each ISOLATION_NOTES as n}<li>{n}</li>{/each}</ul>
    {/if}
    {#if canEdit}
      <div class="row end full">
        {#if s.isolation === 'container' && eng?.available}<button class="btn" type="button" disabled={pulling} onclick={pull}><Icon name="download" size={15} />Download image</button>{/if}
        <button class="btn" type="button" disabled={!dirty} onclick={() => (f = seedOf())}>Reset</button>
        <button class="btn primary" disabled={!dirty || saving || !!hint}>{saving ? 'Saving…' : 'Save isolation'}</button>
      </div>
    {/if}
  </form>
</section>

<style>
  .status { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .lf { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px 16px; margin-top: 16px; align-items: start; }
  .full { grid-column: 1 / -1; }
  .notes { margin: 0; padding-left: 18px; color: var(--muted); font-size: var(--fs-sm); display: grid; gap: 4px; }
  @media (max-width: 860px) { .lf { grid-template-columns: minmax(0, 1fr); } }
</style>
