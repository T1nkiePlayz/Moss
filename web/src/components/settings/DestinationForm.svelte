<script lang="ts">
  import { untrack } from 'svelte';
  import Modal from '../Modal.svelte';
  import Icon from '../Icon.svelte';
  import { ApiError, errMsg, type BackupDestination, type DestinationInput, type DestinationTest } from '../../lib/api';
import { apiOps } from '../../lib/api-ops';
  import { destinationProblems } from '../../lib/remote';
  import { toast } from '../../lib/stores.svelte';

  let { dest = null, onclose, ondone }: { dest?: BackupDestination | null; onclose: () => void; ondone: () => void } = $props();
  const d0 = untrack(() => dest); // the form edits a copy; the modal is recreated for each destination
  const editing = !!d0;
  let f = $state<DestinationInput>(d0
    ? { name: d0.name, kind: d0.kind, path: d0.path ?? '', endpoint: d0.endpoint ?? '', region: d0.region ?? '', bucket: d0.bucket ?? '', prefix: d0.prefix ?? '', path_style: d0.path_style ?? false, keep_local: d0.keep_local, keep_remote: d0.keep_remote, is_default: d0.is_default, access_key: '', secret_key: '' }
    : { name: '', kind: 'folder', path: '', endpoint: '', region: '', bucket: '', prefix: '', path_style: false, keep_local: 0, keep_remote: 14, is_default: false, access_key: '', secret_key: '' });
  let busy = $state(false);
  let testing = $state(false);
  let result = $state<DestinationTest | null>(null);
  let shown = $state(false);
  let server = $state<Record<string, string>>({});
  let top = $state('');
  const local = $derived(shown ? destinationProblems(f, editing) : {});
  const msg = (k: string) => local[k] || server[k] || '';

  // an empty credential means "keep what is saved" when editing, so it is never sent
  function body(): DestinationInput {
    const b: DestinationInput = { ...f, name: f.name?.trim(), keep_local: Number(f.keep_local) || 0, keep_remote: Number(f.keep_remote) || 1 };
    if (!b.access_key) delete b.access_key;
    if (!b.secret_key) delete b.secret_key;
    return b;
  }
  function fail(e: unknown) {
    server = {}; top = '';
    if (e instanceof ApiError && e.fields.length) for (const x of e.fields) server[x.path] = x.message;
    if (!(e instanceof ApiError) || !e.fields.length) top = errMsg(e);
    else top = e.message;
  }
  async function test() {
    shown = true; result = null; server = {}; top = '';
    if (Object.keys(destinationProblems(f, editing)).length) return;
    testing = true;
    try { result = dest && !f.access_key && !f.secret_key && f.kind === dest.kind ? await apiOps.testDestination(dest.id) : await apiOps.testNewDestination(body()); } catch (e) { fail(e); }
    testing = false;
  }
  async function save(e: Event) {
    e.preventDefault(); shown = true; server = {}; top = '';
    if (Object.keys(destinationProblems(f, editing)).length) return;
    busy = true;
    try {
      if (dest) await apiOps.patchDestination(dest.id, { ...body(), kind: undefined }); else await apiOps.addDestination(body());
      toast('success', editing ? 'Destination updated' : 'Destination added', { body: 'Run a test to make sure Moss can write to it.' });
      ondone();
    } catch (er) { fail(er); }
    busy = false;
  }
</script>

<Modal title={editing ? 'Edit backup destination' : 'Add a backup destination'} {onclose} width={600}>
  <form id="destf" class="col" onsubmit={save} novalidate>
    <div class="field"><label for="dn">Name</label><input id="dn" class="input" bind:value={f.name} maxlength="60" placeholder="Home NAS, Backblaze…" aria-invalid={!!msg('name')} data-autofocus />{#if msg('name')}<span class="err" role="alert">{msg('name')}</span>{/if}</div>
    {#if !editing}
      <div class="field"><span class="label" id="dk">Where do backups go?</span>
        <div class="row g6" role="radiogroup" aria-labelledby="dk">
          <button type="button" class="chip" role="radio" aria-checked={f.kind === 'folder'} onclick={() => (f.kind = 'folder')}>A folder on this machine or a mounted disk</button>
          <button type="button" class="chip" role="radio" aria-checked={f.kind === 's3'} onclick={() => (f.kind = 's3')}>S3-compatible storage</button>
        </div></div>
    {/if}
    {#if f.kind === 'folder'}
      <div class="field"><label for="dp">Folder</label><input id="dp" class="input mono" bind:value={f.path} placeholder="/mnt/nas/moss" aria-invalid={!!msg('path')} aria-describedby="dp-h" />
        <span class="hint" id="dp-h">The full path of a folder that already exists, such as a mounted NAS or USB disk. It cannot be inside Moss's own data or server folders.</span>{#if msg('path')}<span class="err" role="alert">{msg('path')}</span>{/if}</div>
    {:else}
      <div class="grid cols-2">
        <div class="field"><label for="de">Service address</label><input id="de" class="input mono" bind:value={f.endpoint} placeholder="https://s3.us-west-004.backblazeb2.com" aria-invalid={!!msg('endpoint')} />{#if msg('endpoint')}<span class="err" role="alert">{msg('endpoint')}</span>{/if}</div>
        <div class="field"><label for="dr">Region</label><input id="dr" class="input mono" bind:value={f.region} placeholder="us-east-1" aria-invalid={!!msg('region')} />{#if msg('region')}<span class="err" role="alert">{msg('region')}</span>{/if}</div>
        <div class="field"><label for="db">Bucket</label><input id="db" class="input mono" bind:value={f.bucket} aria-invalid={!!msg('bucket')} />{#if msg('bucket')}<span class="err" role="alert">{msg('bucket')}</span>{/if}</div>
        <div class="field"><label for="dx">Folder inside the bucket <span class="muted">(optional)</span></label><input id="dx" class="input mono" bind:value={f.prefix} placeholder="minecraft" aria-invalid={!!msg('prefix')} />{#if msg('prefix')}<span class="err" role="alert">{msg('prefix')}</span>{/if}</div>
        <div class="field"><label for="da">Access key</label><input id="da" class="input mono" bind:value={f.access_key} autocomplete="off" spellcheck="false" placeholder={editing && dest?.access_key === 'set' ? 'Saved. Type to replace.' : ''} aria-invalid={!!msg('access_key')} />{#if msg('access_key')}<span class="err" role="alert">{msg('access_key')}</span>{/if}</div>
        <div class="field"><label for="ds">Secret key</label><input id="ds" class="input mono" type="password" bind:value={f.secret_key} autocomplete="new-password" spellcheck="false" placeholder={editing && dest?.secret_key === 'set' ? 'Saved. Type to replace.' : ''} aria-invalid={!!msg('secret_key')} />{#if msg('secret_key')}<span class="err" role="alert">{msg('secret_key')}</span>{/if}</div>
      </div>
      <label class="row g8"><input type="checkbox" class="check" bind:checked={f.path_style} /><span>Use path-style addresses<span class="hint" style="display:block">Turn this on for MinIO and most self-hosted storage.</span></span></label>
      <p class="hint">Keys are stored on this machine in a private file and are never shown again, included in support bundles or sent anywhere except to this service.</p>
    {/if}
    <div class="grid cols-2">
      <div class="field"><label for="kl">Keep on this machine</label><input id="kl" class="input" type="number" min="0" bind:value={f.keep_local} aria-describedby="kl-h" /><span class="hint" id="kl-h">0 keeps every local backup. Otherwise only the newest N stay here once their copy is verified.</span></div>
      <div class="field"><label for="kr">Keep at the destination</label><input id="kr" class="input" type="number" min="1" bind:value={f.keep_remote} aria-describedby="kr-h" /><span class="hint" id="kr-h">Older copies are removed. Locked backups and the newest good copy always stay.</span></div>
    </div>
    <label class="row g8"><input type="checkbox" class="check" bind:checked={f.is_default} /><span>Use for every server unless it has its own choice</span></label>
    {#if top}<div class="banner danger" role="alert"><Icon name="alert" size={16} />{top}</div>{/if}
    {#if result}
      <div class="banner {result.ok ? 'ok' : 'danger'}" role="status"><Icon name={result.ok ? 'check' : 'alert'} size={16} />
        <div><b>{result.ok ? 'Everything worked' : 'The test failed'}</b>
          <ul class="steps">{#each result.steps as s}<li class:bad={!s.ok}>{s.ok ? '✓' : '✗'} {s.step}{s.detail ? `: ${s.detail}` : ''}</li>{/each}</ul></div></div>
    {/if}
  </form>
  {#snippet footer()}
    <button class="btn" type="button" onclick={onclose}>Cancel</button>
    <button class="btn" type="button" onclick={test} disabled={testing || busy}>{testing ? 'Testing…' : 'Test'}</button>
    <button class="btn primary" type="submit" form="destf" disabled={busy || testing}>{busy ? 'Saving…' : editing ? 'Save changes' : 'Add destination'}</button>
  {/snippet}
</Modal>

<style>
  .steps { margin: 6px 0 0; padding: 0; list-style: none; font-size: var(--fs-sm); } .steps li.bad { color: var(--danger); }
  .banner.ok { background: var(--a-soft); border-color: color-mix(in srgb, var(--a2), transparent 60%); }
</style>
