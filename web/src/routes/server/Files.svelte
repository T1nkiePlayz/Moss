<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import States from '../../components/States.svelte';
  import PromptModal from '../../components/PromptModal.svelte';
  import { api, download, fileDownloadPath, errMsg, type FileEntry, type FileList, type Server } from '../../lib/api';
  import { bytes, relTime } from '../../lib/format';
  import { confirmDialog, toast, toastError, undoToast } from '../../lib/stores.svelte';
  import Lazy from '../../components/Lazy.svelte';
  import { can } from '../../lib/auth.svelte';

  let { server: s }: { server: Server } = $props();
  const canWrite = $derived(can('files.write'));

  let view = $state<'browse' | 'config'>('browse');
  const loadConfig = () => import('../../components/ConfigEditor.svelte');
  let path = $state('');
  let listing = $state<FileList | null>(null);
  let err = $state('');
  let loading = $state(true);
  let dragging = $state(false);
  let uploading = $state(false);
  let prompt = $state<null | { kind: 'mkdir' | 'rename'; entry?: FileEntry }>(null);

  // editor
  let open = $state<{ path: string; original: string; size: number } | null>(null);
  let text = $state('');
  let blocked = $state<'binary' | 'truncated' | ''>('');
  let opening = $state(false);
  let saving = $state(false);
  let gutter: HTMLDivElement | undefined = $state();
  const dirty = $derived(!!open && !blocked && text !== open.original);
  const lineCount = $derived(text.split('\n').length);

  const join = (a: string, b: string) => (a ? a + '/' + b : b);
  const crumbs = $derived(path.split('/').filter(Boolean).map((p, i, a) => ({ p, to: a.slice(0, i + 1).join('/') })));

  async function load() {
    loading = true; err = '';
    try { listing = await api.files(s.id, path); } catch (e) { err = errMsg(e); }
    loading = false;
  }
  $effect(() => { s.id; path; load(); });

  async function openFile(e: FileEntry) {
    const p = join(path, e.name);
    opening = true; blocked = '';
    try {
      const c = await api.fileContent(s.id, p);
      open = { path: p, original: c.content, size: c.size };
      text = c.content;
      blocked = c.binary ? 'binary' : c.truncated ? 'truncated' : '';
    } catch (er) { toastError(er, 'Could not open file'); }
    opening = false;
  }
  async function closeEditor() {
    if (dirty && !(await confirmDialog({ title: 'Discard unsaved changes?', body: `${open!.path} has changes that haven't been saved.`, confirmLabel: 'Discard', danger: true })).ok) return;
    open = null;
  }
  async function save() {
    if (!open || !dirty || saving) return;
    saving = true;
    try {
      const r = await api.saveFile(s.id, open.path, text);
      open = { ...open, original: text };
      undoToast(`Saved ${open.path.split('/').pop()}`, r.undo_id, () => open && openFile({ name: open.path.split('/').pop()!, kind: 'file', size: 0, modified: '' }));
    } catch (e) { toastError(e, 'Save failed'); }
    saving = false;
  }
  function editorKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); void save(); }
    else if (e.key === 'Tab') { // insert two spaces instead of leaving the editor
      e.preventDefault();
      const t = e.currentTarget as HTMLTextAreaElement;
      const a = t.selectionStart;
      text = text.slice(0, a) + '  ' + text.slice(t.selectionEnd);
      requestAnimationFrame(() => (t.selectionStart = t.selectionEnd = a + 2));
    }
  }
  $effect(() => {
    if (!dirty) return;
    const h = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener('beforeunload', h);
    return () => window.removeEventListener('beforeunload', h);
  });

  async function upload(files: File[]) {
    if (!files.length || !canWrite) return;
    uploading = true;
    try { await api.upload(s.id, path, files); toast('success', `Uploaded ${files.length} file${files.length > 1 ? 's' : ''}`); await load(); } catch (e) { toastError(e, 'Upload failed'); }
    uploading = false;
  }
  function onDrop(e: DragEvent) { e.preventDefault(); dragging = false; void upload([...(e.dataTransfer?.files ?? [])]); }
  async function submitPrompt(v: string) {
    const p = prompt!;
    try {
      if (p.kind === 'mkdir') await api.mkdir(s.id, join(path, v));
      else await api.rename(s.id, join(path, p.entry!.name), join(path, v));
      prompt = null; await load();
    } catch (e) { toastError(e, p.kind === 'mkdir' ? 'Could not create folder' : 'Rename failed'); }
  }
  async function remove(e: FileEntry) {
    const r = await confirmDialog({ title: `Delete ${e.name}?`, body: e.kind === 'dir' ? 'The folder and everything inside it will be deleted.' : 'This file will be deleted.', consequence: s.state === 'running' ? 'The server is running; deleting live files may break it.' : undefined, confirmLabel: 'Delete', danger: true });
    if (!r.ok) return;
    try { await api.deleteFile(s.id, join(path, e.name)); toast('info', `Deleted ${e.name}`); await load(); } catch (er) { toastError(er, 'Delete failed'); }
  }
  const dl = (p: string) => download(fileDownloadPath(s.id, p), p.split('/').pop()!).catch((e) => toastError(e, 'Download failed'));
  const icon = (e: FileEntry) => (e.kind === 'dir' ? 'folder' : /\.(ya?ml|json|properties|txt|log|toml|cfg|conf|md)$/i.test(e.name) ? 'file-text' : /\.(jar|zip|gz)$/i.test(e.name) ? 'package' : 'file');
  function go(p: string) { path = p; }
</script>

{#if !open}
  <div class="row g6" role="group" aria-label="Files view" style="margin-bottom:12px">
    <button class="chip" aria-pressed={view === 'browse'} onclick={() => (view = 'browse')}><Icon name="folder" size={14} />Browse</button>
    <button class="chip" aria-pressed={view === 'config'} onclick={() => (view = 'config')}><Icon name="sliders" size={14} />Configuration</button>
  </div>
{/if}

{#if !open && view === 'config'}
  <Lazy load={loadConfig} props={{ server: s }} />
{:else if open}
  <section class="card ed">
    <div class="row wrap between">
      <div class="row g6" style="min-width:0">
        <button class="btn ghost sm" onclick={closeEditor}><Icon name="chevron-left" size={14} />Files</button>
        <b class="mono ellipsis">{open.path}</b>
        {#if dirty}<span class="badge warn" role="status"><span class="dot needs_attention"></span>Unsaved</span>{:else if !blocked}<span class="badge ok">Saved</span>{/if}
      </div>
      <div class="row g6">
        <span class="xs faint">{bytes(open.size)}</span>
        <button class="btn sm" onclick={() => dl(open!.path)}><Icon name="download" size={14} />Download</button>
        {#if canWrite && !blocked}<button class="btn primary sm" disabled={!dirty || saving} onclick={save}><Icon name="save" size={14} />{saving ? 'Saving…' : 'Save'} <kbd style="margin-left:4px">Ctrl S</kbd></button>{/if}
      </div>
    </div>
    {#if blocked}
      <States kind="empty" icon="file" title={blocked === 'binary' ? 'Binary file' : 'File too large to edit'} text={blocked === 'binary' ? 'This file is not text, so it cannot be edited here.' : 'Only the first part was loaded, so editing is disabled to protect the file. Download it instead.'}>
        <button class="btn primary" onclick={() => dl(open!.path)}><Icon name="download" size={15} />Download</button>
      </States>
    {:else}
      <div class="editor">
        <div class="gutter mono" bind:this={gutter} aria-hidden="true">{#each { length: lineCount } as _, i}<div>{i + 1}</div>{/each}</div>
        <textarea class="mono" wrap="off" spellcheck="false" aria-label="Editing {open.path}" bind:value={text} readonly={!canWrite} onkeydown={editorKey} onscroll={(e) => gutter && (gutter.scrollTop = e.currentTarget.scrollTop)}></textarea>
      </div>
    {/if}
  </section>
{:else}
  <div class="row between wrap" style="margin-bottom:12px">
    <nav aria-label="Path" class="row g4 wrap crumbs">
      <button class="btn ghost sm" onclick={() => go('')} aria-label="Server root"><Icon name="home" size={14} />root</button>
      {#each crumbs as c}<Icon name="chevron-right" size={13} /><button class="btn ghost sm" onclick={() => go(c.to)}>{c.p}</button>{/each}
    </nav>
    {#if canWrite}
      <div class="row g6">
        <button class="btn sm" onclick={() => (prompt = { kind: 'mkdir' })}><Icon name="plus" size={14} />New folder</button>
        <label class="btn sm primary" style="cursor:pointer"><Icon name="upload" size={14} />{uploading ? 'Uploading…' : 'Upload'}<input type="file" multiple class="sr" onchange={(e) => { upload([...(e.currentTarget.files ?? [])]); e.currentTarget.value = ''; }} /></label>
      </div>
    {/if}
  </div>

  <div class="drop" class:dragging role="region" aria-label="File list (drop files here to upload)" ondragover={(e) => { if (canWrite) { e.preventDefault(); dragging = true; } }} ondragleave={() => (dragging = false)} ondrop={onDrop}>
    {#if err}<States kind="error" text={err} retry={load} />
    {:else if loading && !listing}<States kind="loading" rows={5} />
    {:else if listing && !listing.entries.length}<States kind="empty" icon="folder" title="Empty folder" text={canWrite ? 'Drag files here to upload them.' : undefined} />
    {:else if listing}
      <div class="table-wrap" aria-busy={loading || opening}>
        <table>
          <thead><tr><th>Name</th><th class="num">Size</th><th>Modified</th><th></th></tr></thead>
          <tbody>
            {#if path}<tr><td colspan="4"><button class="btn ghost sm" onclick={() => go(path.split('/').slice(0, -1).join('/'))}><Icon name="arrow-up" size={14} />..</button></td></tr>{/if}
            {#each listing.entries as e (e.name)}
              <tr>
                <td><button class="name" onclick={() => (e.kind === 'dir' ? go(join(path, e.name)) : openFile(e))}><span class="fi {e.kind}"><Icon name={icon(e)} size={16} /></span><span class="ellipsis">{e.name}</span></button></td>
                <td class="num muted">{e.kind === 'dir' ? '—' : bytes(e.size)}</td>
                <td class="muted">{relTime(e.modified)}</td>
                <td class="num">
                  <span class="acts">
                    {#if e.kind === 'file'}<button class="btn ghost icon sm" aria-label="Download {e.name}" onclick={() => dl(join(path, e.name))}><Icon name="download" size={14} /></button>{/if}
                    {#if canWrite}
                      <button class="btn ghost icon sm" aria-label="Rename {e.name}" onclick={() => (prompt = { kind: 'rename', entry: e })}><Icon name="edit" size={14} /></button>
                      <button class="btn ghost icon sm" aria-label="Delete {e.name}" onclick={() => remove(e)}><Icon name="trash" size={14} /></button>
                    {/if}
                  </span>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
    {#if dragging}<div class="veil"><Icon name="upload" size={28} />Drop to upload to /{path}</div>{/if}
  </div>
{/if}

{#if prompt}
  <PromptModal title={prompt.kind === 'mkdir' ? 'New folder' : `Rename ${prompt.entry!.name}`} label={prompt.kind === 'mkdir' ? 'Folder name' : 'New name'} value={prompt.entry?.name ?? ''} confirmLabel={prompt.kind === 'mkdir' ? 'Create' : 'Rename'} onsubmit={submitPrompt} oncancel={() => (prompt = null)} />
{/if}

<style>
  .drop { position: relative; }
  .veil { position: absolute; inset: 0; z-index: 3; display: flex; flex-direction: column; gap: 8px; align-items: center; justify-content: center; background: color-mix(in srgb, var(--bg), transparent 15%); border: 2px dashed var(--a1); border-radius: var(--r); color: var(--a-text); font-weight: 600; }
  .name { display: flex; align-items: center; gap: 10px; background: none; border: 0; cursor: pointer; color: var(--text); font-weight: 500; padding: 0; height: var(--row); max-width: 360px; }
  .name:hover { color: var(--a-text); }
  .fi { color: var(--muted); display: grid; } .fi.dir { color: var(--a-text); }
  .acts { opacity: 0; transition: opacity var(--t); white-space: nowrap; } tr:hover .acts, tr:focus-within .acts { opacity: 1; }
  @media (hover: none) { .acts { opacity: 1; } }
  .ed { display: flex; flex-direction: column; gap: 12px; padding: 14px; }
  .editor { display: flex; height: calc(100dvh - 330px); min-height: 320px; background: var(--bg); border: 1px solid var(--line2); border-radius: var(--r-sm); overflow: hidden; }
  .editor:focus-within { border-color: var(--a1); box-shadow: 0 0 0 3px var(--a-soft); }
  .gutter { width: 52px; flex: none; overflow: hidden; text-align: right; padding: 12px 10px 12px 0; color: var(--faint); background: var(--s1); border-right: 1px solid var(--line); font-size: 12.5px; line-height: 20px; user-select: none; }
  textarea { flex: 1; min-width: 0; resize: none; border: 0; outline: 0; background: transparent; color: var(--text); padding: 12px 14px; font-size: 12.5px; line-height: 20px; white-space: pre; overflow: auto; tab-size: 2; }
  .crumbs { min-width: 0; }
</style>
