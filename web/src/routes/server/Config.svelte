<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import States from '../../components/States.svelte';
  import { api, errMsg, type Server } from '../../lib/api';
  import { PROP_GROUPS, parseProps, setProp } from '../../lib/props';
  import { undoToast, toastError } from '../../lib/stores.svelte';
  import { can } from '../../lib/auth.svelte';

  let { server: s }: { server: Server } = $props();
  let raw = $state('');
  let original = $state('');
  let loaded = $state(false);
  let err = $state('');
  let rawMode = $state(false);
  let saving = $state(false);
  const canWrite = $derived(can('config.write'));

  async function load() {
    err = ''; loaded = false;
    try { const r = await api.properties(s.id); raw = original = r.raw; } catch (e) { err = errMsg(e); }
    loaded = true;
  }
  $effect(() => { s.id; load(); });

  const cur = $derived(new Map(parseProps(raw).map((e) => [e.key, e.value])));
  const orig = $derived(new Map(parseProps(original).map((e) => [e.key, e.value])));
  const known = new Set(PROP_GROUPS.flatMap((g) => g.items.map((i) => i.key)));
  const extra = $derived([...cur.keys()].filter((k) => !known.has(k)));
  const changed = $derived([...new Set([...cur.keys(), ...orig.keys()])].filter((k) => cur.get(k) !== orig.get(k)));
  const dirty = $derived(raw !== original);
  const needsRestart = $derived(changed.length > 0 && (s.state === 'running' || s.state === 'starting'));
  const set = (k: string, v: string) => (raw = setProp(raw, k, v));

  async function save() {
    saving = true;
    try {
      const r = await api.saveProperties(s.id, raw);
      original = raw;
      undoToast('server.properties saved', r.undo_id, load);
    } catch (e) { toastError(e, 'Save failed'); }
    saving = false;
  }
</script>

{#if err}
  <States kind="error" text={err} retry={load} />
{:else if !loaded}
  <States kind="loading" rows={4} />
{:else}
  <div class="row between wrap" style="margin-bottom:16px">
    <div class="row g6" role="group" aria-label="Editor mode">
      <button class="chip" aria-pressed={!rawMode} onclick={() => (rawMode = false)}><Icon name="sliders" size={14} />Form</button>
      <button class="chip" aria-pressed={rawMode} onclick={() => (rawMode = true)}><Icon name="file-text" size={14} />Raw</button>
    </div>
    <div class="row g6">
      {#if dirty}<span class="badge warn" role="status">{changed.length} change{changed.length === 1 ? '' : 's'}</span><button class="btn ghost sm" onclick={() => (raw = original)}>Discard</button>{/if}
      {#if canWrite}<button class="btn primary" disabled={!dirty || saving} onclick={save}><Icon name="save" size={15} />{saving ? 'Saving…' : 'Save'}</button>{/if}
    </div>
  </div>

  {#if needsRestart}<div class="banner warn" style="margin-bottom:16px"><Icon name="refresh" size={16} />The server is running. Changes to <code>server.properties</code> take effect after a restart.</div>{/if}

  {#if rawMode}
    <textarea class="textarea mono" style="min-height:420px;white-space:pre;overflow:auto" wrap="off" spellcheck="false" aria-label="server.properties" bind:value={raw} readonly={!canWrite}></textarea>
  {:else}
    <div class="col" style="gap:var(--gap)">
      {#each PROP_GROUPS as g}
        <section class="card">
          <div class="card-title">{g.title}</div>
          <div class="grid fields">
            {#each g.items as d (d.key)}
              {@const v = cur.get(d.key)}
              {#if v !== undefined}
                <div class="field" class:bool={d.type === 'bool'}>
                  {#if d.type === 'bool'}
                    <label class="row" style="justify-content:space-between;cursor:pointer;color:var(--text)">
                      <span>{d.label}{#if d.restart}<span class="xs faint"> · restart</span>{/if}<span class="xs faint mono" style="display:block">{d.key}</span></span>
                      <input type="checkbox" class="switch" checked={v === 'true'} disabled={!canWrite} onchange={(e) => set(d.key, e.currentTarget.checked ? 'true' : 'false')} />
                    </label>
                  {:else}
                    <label for="p-{d.key}">{d.label}{#if d.restart}<span class="xs faint"> · restart</span>{/if}</label>
                    {#if d.type === 'select'}
                      <select id="p-{d.key}" class="select" value={v} disabled={!canWrite} onchange={(e) => set(d.key, e.currentTarget.value)}>
                        {#each d.options! as o}<option>{o}</option>{/each}
                        {#if !d.options!.includes(v)}<option>{v}</option>{/if}
                      </select>
                    {:else}
                      <input id="p-{d.key}" class="input" type={d.type === 'number' ? 'number' : 'text'} min={d.min} max={d.max} value={v} readonly={!canWrite} oninput={(e) => set(d.key, e.currentTarget.value)} />
                    {/if}
                    <span class="hint mono">{d.key}</span>{#if d.hint}<span class="hint">{d.hint}</span>{/if}
                  {/if}
                </div>
              {/if}
            {/each}
          </div>
        </section>
      {/each}
      {#if extra.length}
        <details class="card">
          <summary class="card-title" style="cursor:pointer;margin:0">Other properties ({extra.length})</summary>
          <div class="grid fields" style="margin-top:14px">
            {#each extra as k (k)}
              <div class="field"><label for="p-{k}" class="mono">{k}</label><input id="p-{k}" class="input" value={cur.get(k)} readonly={!canWrite} oninput={(e) => set(k, e.currentTarget.value)} /></div>
            {/each}
          </div>
        </details>
      {/if}
    </div>
  {/if}
{/if}

<style>
  .fields { grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr)); gap: 16px 24px; }
  .field.bool { justify-content: center; padding: 6px 0; }
</style>
