<script lang="ts">
  import Icon from '../Icon.svelte';
  import { bytes } from '../../lib/format';
  import { modpackSummary } from '../../lib/content';
  import type { ModpackInspect } from '../../lib/api';
  let { pack }: { pack: ModpackInspect } = $props();
</script>

<div class="mp">
  <div class="row between wrap" style="align-items:flex-start">
    <div class="grow">
      <h3>{pack.name} <span class="muted sm tnum">{pack.version}</span></h3>
      {#if pack.summary}<p class="sm muted">{pack.summary}</p>{/if}
    </div>
    <span class="badge">{pack.format === 'mrpack' ? 'Modrinth .mrpack' : 'CurseForge'}</span>
  </div>
  <dl class="facts">
    <div><dt>Game version</dt><dd class="tnum">{pack.game_version}</dd></div>
    <div><dt>Loader</dt><dd>{pack.loader.id}{pack.loader.version ? ` ${pack.loader.version}` : ''}</dd></div>
    <div><dt>Files</dt><dd class="tnum">{pack.files.total}</dd></div>
    <div><dt>Server</dt><dd class="tnum">{pack.files.server}</dd></div>
    <div><dt>Client-only</dt><dd class="tnum">{pack.files.client_only}</dd></div>
    <div class:bad={pack.files.unavailable > 0}><dt>Unavailable</dt><dd class="tnum">{pack.files.unavailable}</dd></div>
    <div><dt>Overrides</dt><dd class="tnum">{pack.overrides.files} · {bytes(pack.overrides.size)}</dd></div>
    <div><dt>Download</dt><dd class="tnum">{bytes(pack.size)}</dd></div>
  </dl>
  <p class="xs muted">{modpackSummary(pack)}. Client-only files are not installed on the server.</p>
  {#if pack.unavailable.length}
    <div class="banner warn" style="flex-direction:column;gap:6px">
      <div class="row g6"><Icon name="alert" size={16} /><b>{pack.unavailable.length} file{pack.unavailable.length === 1 ? '' : 's'} cannot be downloaded automatically</b></div>
      <p class="xs muted">The authors disabled third-party downloads. Download them from the project page and upload them in Files after the import.</p>
      <ul class="un">
        {#each pack.unavailable as u}
          <li><span class="grow ellipsis" title={u.name}>{u.name}</span><span class="xs faint">{u.reason}</span>{#if u.url}<a class="xs" href={u.url} target="_blank" rel="noreferrer noopener">Project page<Icon name="external" size={11} /></a>{/if}</li>
        {/each}
      </ul>
    </div>
  {/if}
  {#each pack.warnings as w}<div class="banner warn"><Icon name="alert" size={16} />{w}</div>{/each}
</div>

<style>
  .mp { display: flex; flex-direction: column; gap: 10px; }
  .facts { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: 8px; margin: 0; }
  .facts div { padding: 8px 10px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); min-width: 0; }
  .facts .bad { border-color: color-mix(in srgb, var(--warn), transparent 55%); background: var(--warn-soft); }
  dt { font-size: var(--fs-xs); color: var(--muted); } dd { margin: 0; font-weight: 600; overflow-wrap: anywhere; }
  .un { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; max-height: 160px; overflow: auto; }
  .un li { display: flex; gap: 10px; align-items: center; }
</style>
