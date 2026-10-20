<script lang="ts">
  import Modal from '../Modal.svelte';
  import Icon from '../Icon.svelte';
  import ModpackPicker from './ModpackPicker.svelte';
  import { api, errMsg, type ModpackInspect, type Operation, type Server } from '../../lib/api';

  let { server: s, onclose, onstarted }: { server: Server; onclose: () => void; onstarted: (o: Operation) => void } = $props();
  let pack = $state<ModpackInspect | null>(null);
  let mode = $state<'add' | 'replace'>('add');
  let busy = $state(false);
  let err = $state('');
  const mismatch = $derived(!!pack && pack.game_version !== s.game_version);
  const runtimeMismatch = $derived(!!pack && pack.loader.id !== s.runtime);

  async function go() {
    if (!pack) return;
    busy = true; err = '';
    try { onstarted((await api.importModpack(s.id, pack.token, mode)).operation); } catch (e) { err = errMsg(e); }
    busy = false;
  }
</script>

<Modal title="Import modpack" width={640} {onclose}>
  <ModpackPicker onresult={(m) => { pack = m; err = ''; }} />
  {#if pack}
    <fieldset class="mode">
      <legend class="label">How should it be imported?</legend>
      <label class="opt"><input type="radio" name="mp-mode" value="add" bind:group={mode} /><span><b>Add to this server</b><span class="xs muted">Keeps what is installed; files with the same name are replaced.</span></span></label>
      <label class="opt"><input type="radio" name="mp-mode" value="replace" bind:group={mode} /><span><b>Replace current {s.content_kind === 'mods' ? 'mods' : 'content'}</b><span class="xs muted">Removes installed files the pack does not contain. Worlds are never touched.</span></span></label>
    </fieldset>
    {#if mismatch || runtimeMismatch}
      <div class="banner warn" style="margin-top:10px"><Icon name="alert" size={16} /><div>This pack targets {pack.loader.id} {pack.game_version}, but this server runs {s.runtime} {s.game_version}. Mods may not load. Use the Upgrade tab to change the version first.</div></div>
    {/if}
    <div class="banner info" style="margin-top:10px"><Icon name="archive" size={16} />A backup is taken first when "Back up before risky changes" is on. You can undo the import afterwards.</div>
  {/if}
  {#if err}<div class="banner danger" style="margin-top:10px" role="alert"><Icon name="x-circle" size={16} />{err}</div>{/if}
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn {mode === 'replace' ? 'danger solid' : 'primary'}" disabled={!pack || busy} onclick={go}>{busy ? 'Starting…' : mode === 'replace' ? 'Replace and import' : 'Import modpack'}</button>
  {/snippet}
</Modal>

<style>
  .mode { border: 0; margin: 16px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .opt { display: flex; gap: 10px; align-items: flex-start; padding: 10px 12px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); cursor: pointer; }
  .opt:has(input:checked) { border-color: var(--a2); background: var(--a-soft); }
  .opt span { display: flex; flex-direction: column; }
  .opt input { margin-top: 3px; accent-color: var(--a2); }
</style>
