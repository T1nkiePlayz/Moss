<script lang="ts">
  import Icon from './Icon.svelte';
  import States from './States.svelte';
  import { api, errMsg, type ConfigDiff, type ConfigField, type ConfigFile, type ConfigFileInfo, type Server } from '../lib/api';
  import { changeCount, diffLines } from '../lib/diff';
  import { relTime } from '../lib/format';
  import { confirmDialog, toastError, undoToast } from '../lib/stores.svelte';
  import { can } from '../lib/auth.svelte';

  let { server: s }: { server: Server } = $props();
  const canWrite = $derived(can('config.write'));

  let files = $state<ConfigFileInfo[]>([]);
  let fileId = $state('');
  let data = $state<ConfigFile | null>(null);
  let err = $state('');
  let loading = $state(true);
  let mode = $state<'form' | 'raw' | 'history'>('form');
  let edits = $state<Record<string, unknown>>({});
  let reset = $state<Set<string>>(new Set());
  let raw = $state('');
  let saving = $state(false);
  let diff = $state<ConfigDiff | null>(null);
  let diffId = $state('');
  let diffErr = $state('');

  async function loadFiles() {
    loading = true; err = '';
    try {
      files = (await api.configFiles(s.id)).files;
      if (!files.some((f) => f.id === fileId)) fileId = files[0]?.id ?? '';
    } catch (e) { err = errMsg(e); }
    loading = false;
  }
  async function loadFile() {
    if (!fileId) { data = null; return; }
    err = '';
    try { data = await api.configFile(s.id, fileId); raw = data.raw; edits = {}; reset = new Set(); diff = null; if (!data.exists) mode = 'form'; } catch (e) { err = errMsg(e); data = null; }
  }
  $effect(() => { s.id; loadFiles(); });
  $effect(() => { s.id; fileId; loadFile(); });

  const groups = $derived.by(() => {
    const m = new Map<string, ConfigField[]>();
    for (const f of data?.fields ?? []) (m.get(f.group) ?? m.set(f.group, []).get(f.group)!).push(f);
    return [...m];
  });
  const value = (f: ConfigField) => (f.key in edits ? edits[f.key] : f.present ? f.value : f.default);
  const changes = $derived(Object.keys(edits).length + reset.size);
  const rawDirty = $derived(!!data && raw !== data.raw);
  const needsRestart = $derived((data?.fields ?? []).some((f) => f.restart && (f.key in edits || reset.has(f.key))) && (s.state === 'running' || s.state === 'starting'));
  const fieldErr = (f: ConfigField, v: unknown): string => {
    if (f.type === 'int' || f.type === 'float') {
      if (typeof v !== 'number' || !isFinite(v)) return 'Enter a number.';
      if (f.type === 'int' && !Number.isInteger(v)) return 'Enter a whole number.';
      if (f.min != null && v < f.min) return `Must be at least ${f.min}.`;
      if (f.max != null && v > f.max) return `Must be at most ${f.max}.`;
    }
    return '';
  };
  const problems = $derived((data?.fields ?? []).filter((f) => f.key in edits && fieldErr(f, edits[f.key])).length);

  function setNum(f: ConfigField, text: string) { edits[f.key] = text.trim() === '' ? NaN : Number(text); reset.delete(f.key); }
  function setVal(f: ConfigField, v: unknown) { if (!f.present && v === f.default) delete edits[f.key]; else edits[f.key] = v; reset.delete(f.key); }
  function revertKey(f: ConfigField) { delete edits[f.key]; reset = new Set([...reset].filter((k) => k !== f.key)); }
  function useDefault(f: ConfigField) { delete edits[f.key]; reset = new Set([...reset, f.key]); }

  async function save() {
    if (!data || saving) return;
    saving = true;
    try {
      const body = mode === 'raw' ? { raw } : { set: edits, unset: [...reset] };
      if (mode !== 'raw') for (const k of Object.keys(edits)) if (typeof edits[k] === 'number' && isNaN(edits[k] as number)) throw new Error('Fix the highlighted values first.');
      const r = await api.saveConfig(s.id, data.id, body);
      if (!r.changed) toastError(new Error('Nothing changed.'), 'Nothing to save');
      else if (r.undo_id) undoToast(`${data.title} saved`, r.undo_id, loadFile);
      await loadFile();
    } catch (e) { toastError(e, 'Save failed'); }
    saving = false;
  }
  async function discard() { edits = {}; reset = new Set(); raw = data?.raw ?? ''; }

  async function showDiff(id?: string) {
    if (!data) return;
    diffErr = ''; diff = null; diffId = id ?? '';
    try { diff = await api.configDiff(s.id, data.id, id); } catch (e) { diffErr = errMsg(e); }
  }
  $effect(() => { if (mode === 'history' && data && !diff && !diffErr) showDiff(); });
  async function restore(id: string, title: string) {
    const r = await confirmDialog({ title: 'Restore this version?', body: `The file goes back to how it was before "${title}". Your current version is kept in the history too.`, confirmLabel: 'Restore' });
    if (!r.ok) return;
    try { await api.revert(id); await loadFile(); } catch (e) { toastError(e, 'Could not restore'); }
  }
  const lines = $derived(diff ? diffLines(diff.previous ?? '', diff.current) : []);
  const display = (f: ConfigField) => (f.secret ? 'hidden' : String(value(f)));
</script>

{#if err && !data && !files.length}
  <States kind="error" text={err} retry={loadFiles} />
{:else if loading}
  <States kind="loading" rows={4} />
{:else if !files.length}
  <States kind="empty" icon="sliders" title="No configuration files to edit" text="This kind of server has no files the structured editors cover. Use the file browser for everything else." />
{:else}
  <div class="row between wrap" style="margin-bottom:16px;gap:12px">
    <div class="row g6 wrap" role="group" aria-label="Configuration file">
      {#each files as f (f.id)}<button class="chip" aria-pressed={fileId === f.id} onclick={() => (fileId = f.id)}>{f.title}{#if !f.exists}<span class="xs"> · not created</span>{/if}</button>{/each}
    </div>
    <div class="row g6" role="group" aria-label="Editor mode">
      <button class="chip" aria-pressed={mode === 'form'} onclick={() => (mode = 'form')}><Icon name="sliders" size={14} />Form</button>
      <button class="chip" aria-pressed={mode === 'raw'} onclick={() => (mode = 'raw')}><Icon name="file-text" size={14} />Raw</button>
      <button class="chip" aria-pressed={mode === 'history'} onclick={() => (mode = 'history')}><Icon name="clock" size={14} />History{#if data?.history.length}<span class="xs"> {data.history.length}</span>{/if}</button>
    </div>
  </div>

  {#if !data}
    {#if err}<States kind="error" text={err} retry={loadFile} />{:else}<States kind="loading" rows={3} />{/if}
  {:else if !data.exists}
    <States kind="empty" icon="file" title="{data.title} has not been created yet" text="Minecraft writes {data.path} the first time the server starts. Start the server once, then come back." />
  {:else}
    {#if data.parse_error}<div class="banner danger" style="margin-bottom:16px" role="alert"><Icon name="alert" size={16} /><span>This file has a syntax problem, so the form is disabled: {data.parse_error}. Fix it in the raw editor.</span></div>{/if}

    {#if mode !== 'history'}
      <div class="row between wrap" style="margin-bottom:16px">
        <span class="xs faint mono">{data.path}</span>
        <div class="row g6">
          {#if (mode === 'raw' ? rawDirty : changes > 0)}<span class="badge warn" role="status">{mode === 'raw' ? 'Unsaved changes' : `${changes} change${changes === 1 ? '' : 's'}`}</span><button class="btn ghost sm" onclick={discard}>Discard</button>{/if}
          {#if canWrite}<button class="btn primary" disabled={saving || (mode === 'raw' ? !rawDirty : changes === 0 || problems > 0)} onclick={save}><Icon name="save" size={15} />{saving ? 'Saving…' : 'Save'}</button>{/if}
        </div>
      </div>
      {#if needsRestart}<div class="banner warn" style="margin-bottom:16px"><Icon name="refresh" size={16} />The server is running. Some of these changes take effect after a restart.</div>{/if}
    {/if}

    {#if mode === 'raw'}
      <p class="muted sm" style="margin:0 0 8px">Your comments and layout are kept exactly as you write them. A copy of the old file is saved so you can go back.</p>
      <textarea class="textarea mono" style="min-height:420px;white-space:pre;overflow:auto" wrap="off" spellcheck="false" aria-label="{data.path}" bind:value={raw} readonly={!canWrite}></textarea>
    {:else if mode === 'history'}
      <div class="grid hist">
        <section class="card">
          <div class="card-title">Saved versions</div>
          {#if !data.history.length}<p class="muted sm">No earlier versions yet. One is saved every time you change this file here.</p>{/if}
          <ul class="list">
            {#each data.history as h (h.id)}
              <li class="row between" style="gap:8px">
                <button class="linkish" aria-pressed={diff?.snapshot?.id === h.id || (!diffId && diff?.snapshot?.id === h.id)} onclick={() => showDiff(h.id)}><b>{h.title}</b><span class="xs faint" style="display:block">{relTime(h.created_at)}</span></button>
                {#if canWrite}<button class="btn sm" onclick={() => restore(h.id, h.title)}>Restore</button>{/if}
              </li>
            {/each}
          </ul>
        </section>
        <section class="card">
          <div class="card-title">What changed {#if diff?.snapshot}<span class="xs faint">since {relTime(diff.snapshot.created_at)}</span>{/if}</div>
          {#if diffErr}<States kind="error" text={diffErr} retry={() => showDiff(diffId || undefined)} />
          {:else if !diff}<States kind="loading" rows={3} />
          {:else if !diff.snapshot}<p class="muted sm">Nothing to compare yet.</p>
          {:else if changeCount(lines) === 0}<p class="muted sm">This version is the same as the current file.</p>
          {:else}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <div class="diff mono" role="region" aria-label="Differences between the saved version and the current file" tabindex="0">
              {#each lines as l}<div class="dl {l.kind}"><span class="sign" aria-hidden="true">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ' '}</span><span class="sr">{l.kind === 'add' ? 'Added: ' : l.kind === 'del' ? 'Removed: ' : ''}</span>{l.text || ' '}</div>{/each}
            </div>
          {/if}
        </section>
      </div>
    {:else if !data.parse_error}
      <div class="col" style="gap:var(--gap)">
        {#each groups as [g, fs] (g)}
          <section class="card">
            <div class="card-title">{g}</div>
            <div class="grid fields">
              {#each fs as f (f.key)}
                {@const v = value(f)}
                {@const bad = f.key in edits ? fieldErr(f, v) : ''}
                <div class="field" class:bool={f.type === 'bool'}>
                  {#if f.type === 'bool'}
                    <label class="row" style="justify-content:space-between;cursor:pointer;color:var(--text);gap:12px">
                      <span>{f.key.split('.').pop()}{#if f.restart}<span class="xs faint"> · restart</span>{/if}</span>
                      <input type="checkbox" class="switch" checked={v === true} disabled={!canWrite} onchange={(e) => setVal(f, e.currentTarget.checked)} />
                    </label>
                  {:else}
                    <label for="c-{f.key}">{f.key.split('.').pop()}{#if f.restart}<span class="xs faint"> · restart</span>{/if}</label>
                    {#if f.type === 'enum'}
                      <select id="c-{f.key}" class="select" value={String(v)} disabled={!canWrite} onchange={(e) => setVal(f, e.currentTarget.value)}>
                        {#each f.options as o}<option>{o}</option>{/each}{#if !f.options.includes(String(v))}<option>{String(v)}</option>{/if}
                      </select>
                    {:else if f.type === 'int' || f.type === 'float'}
                      <input id="c-{f.key}" class="input" type="number" step={f.type === 'int' ? 1 : 'any'} min={f.min ?? undefined} max={f.max ?? undefined} value={typeof v === 'number' && isNaN(v) ? '' : String(v)} readonly={!canWrite} aria-invalid={!!bad} aria-describedby="d-{f.key}" oninput={(e) => setNum(f, e.currentTarget.value)} />
                    {:else}
                      <input id="c-{f.key}" class="input" type={f.secret ? 'password' : 'text'} placeholder={f.secret ? 'Hidden. Type to replace.' : ''} value={f.secret ? '' : String(v ?? '')} readonly={!canWrite} aria-describedby="d-{f.key}" oninput={(e) => setVal(f, e.currentTarget.value)} />
                    {/if}
                  {/if}
                  <span class="hint" id="d-{f.key}">{f.description}</span>
                  {#if bad}<span class="hint err" role="alert">{bad}</span>{/if}
                  {#if f.danger}<span class="hint warnt"><Icon name="alert" size={12} /> Changing this can break things; read the note first.</span>{/if}
                  <span class="hint mono">{f.key}{#if !f.present && !(f.key in edits)} · not in the file, default {display({ ...f, present: true, value: f.default })}{/if}
                    {#if canWrite && (f.key in edits || reset.has(f.key))} · <button class="linkish inline" onclick={() => revertKey(f)}>undo</button>{:else if canWrite && f.present && f.value !== f.default} · <button class="linkish inline" onclick={() => useDefault(f)}>use default ({String(f.default)})</button>{/if}
                  </span>
                </div>
              {/each}
            </div>
          </section>
        {/each}
        <p class="muted sm" style="margin:0">Only settings this editor knows are shown. Everything else in the file stays as it is. Use the Raw editor for any other setting.</p>
      </div>
    {/if}
  {/if}
{/if}

<style>
  .fields { grid-template-columns: repeat(auto-fill, minmax(min(100%, 300px), 1fr)); gap: 16px 24px; }
  .field.bool { justify-content: center; padding: 6px 0; }
  .hint.err { color: var(--danger); } .hint.warnt { color: var(--warn); }
  .hist { grid-template-columns: minmax(240px, 1fr) minmax(0, 2fr); gap: var(--gap); align-items: start; }
  @media (max-width: 800px) { .hist { grid-template-columns: 1fr; } }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
  .linkish { background: none; border: 0; padding: 0; color: var(--text); cursor: pointer; text-align: left; font: inherit; }
  .linkish:hover, .linkish[aria-pressed='true'] { color: var(--a-text); }
  .linkish.inline { color: var(--a-text); text-decoration: underline; font-size: inherit; }
  .diff { max-height: 460px; overflow: auto; font-size: 12.5px; line-height: 20px; border: 1px solid var(--line2); border-radius: var(--r-sm); background: var(--bg); }
  .dl { white-space: pre; padding: 0 12px 0 0; display: flex; }
  .dl .sign { width: 24px; flex: none; text-align: center; color: var(--muted); user-select: none; }
  .dl.add { background: var(--ok-soft); } .dl.del { background: var(--danger-soft); }
</style>
