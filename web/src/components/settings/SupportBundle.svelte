<script lang="ts">
  import Icon from '../Icon.svelte';
  import States from '../States.svelte';
  import { apiOps, errMsg, type SupportPreview } from '../../lib/api';
  import { toast, toastError } from '../../lib/stores.svelte';
  import { bytes } from '../../lib/format';

  let preview = $state<SupportPreview | null>(null);
  let loading = $state(false);
  let err = $state('');
  let dl = $state(false);
  const total = $derived(preview ? preview.files.reduce((n, x) => n + x.size, 0) : 0);

  async function load() { loading = true; err = ''; try { preview = await apiOps.supportPreview(); } catch (e) { err = errMsg(e); } loading = false; }
  async function download() {
    dl = true;
    try { await apiOps.downloadSupportBundle(); toast('success', 'Support bundle downloaded', { body: 'Review it before sharing it publicly.' }); } catch (e) { toastError(e, 'Download failed'); }
    dl = false;
  }
</script>

<section class="card" aria-labelledby="sb">
  <div class="card-title" id="sb"><Icon name="package" size={14} />Support bundle</div>
  <p class="muted sm">A zip with version, health, settings and recent logs to attach to a bug report. See exactly what goes in before you download it.</p>
  <div class="row wrap" style="margin-top:12px">
    <button class="btn" onclick={load} disabled={loading}><Icon name="eye" size={15} />{loading ? 'Loading…' : preview ? 'Refresh preview' : 'Preview contents'}</button>
    <button class="btn primary" onclick={download} disabled={dl || !preview} title={preview ? '' : 'Preview the contents first'}><Icon name="download" size={15} />{dl ? 'Preparing…' : 'Download bundle'}</button>
  </div>
  <div aria-live="polite" style="margin-top:14px">
    {#if err}<States kind="error" text={err} retry={load} />
    {:else if preview}
      <div class="table-wrap">
        <table>
          <thead><tr><th>File</th><th class="num">Size</th></tr></thead>
          <tbody>
            {#each preview.files as x (x.name)}<tr><td><code class="xs">{x.name}</code></td><td class="num tnum">{bytes(x.size)}</td></tr>{/each}
            {#if !preview.files.length}<tr><td colspan="2" class="muted">Nothing to include.</td></tr>{/if}
          </tbody>
          <tfoot><tr><td><b>Total (uncompressed)</b></td><td class="num tnum"><b>{bytes(total)}</b></td></tr></tfoot>
        </table>
      </div>
      <div class="banner info" style="margin-top:12px"><Icon name="shield" size={16} />
        <div><b>What is redacted</b>
          {#if preview.redactions.length}<ul>{#each preview.redactions as r}<li>{r}</li>{/each}</ul>{:else}<div class="muted">The backend reported no redaction rules.</div>{/if}
        </div>
      </div>
    {/if}
  </div>
</section>

<style>
  ul { margin: 4px 0 0; padding-left: 18px; } li { color: var(--muted); }
  tfoot td { height: var(--row); padding: 0 14px; border-top: 1px solid var(--line2); }
</style>
