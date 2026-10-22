<script lang="ts">
  import Icon from './Icon.svelte';
  import { api, ApiError, errMsg, type Blueprint, type Operation, type Server } from '../lib/api';
  import { isAdmin } from '../lib/auth.svelte';
  import { maxMemoryMb } from '../lib/stores.svelte';
  import { cleanCode } from '../lib/blueprint-share';

  let { oncreated }: { oncreated: (r: { server: Server; operation: Operation }) => void } = $props();
  let list = $state<Blueprint[]>([]);
  let open = $state(false);
  let id = $state('');
  let name = $state('');
  let eula = $state(false);
  let busy = $state(false);
  let err = $state('');
  $effect(() => { if (isAdmin()) api.blueprints().then((b) => (list = b), () => (list = [])); });
  const bp = $derived(list.find((b) => b.id === id));
  let code = $state('');
  let codeErr = $state('');
  async function useCode() {
    codeErr = '';
    try { const b = await api.importBlueprintCode(cleanCode(code)); list = [b, ...list.filter((x) => x.id !== b.id)]; id = b.id; name = name || b.name.slice(0, 48); code = ''; } catch (e) { codeErr = errMsg(e); }
  }
  async function go() {
    if (!bp) return;
    busy = true; err = '';
    try { oncreated(await api.createFromBlueprint(bp.id, { name: name.trim(), accept_eula: eula })); } catch (e) { err = e instanceof ApiError && e.fields.length ? e.fields.map((f) => f.message).join(' ') : errMsg(e); }
    busy = false;
  }
</script>

{#if list.length || isAdmin()}
  <h3 class="cat">Start from a blueprint</h3>
  <section class="card col" style="gap:12px;margin-bottom:20px">
    {#if !open}
      <div class="row between wrap"><span class="sm muted">{list.length ? `${list.length} saved ${list.length === 1 ? 'blueprint' : 'blueprints'}: same runtime, settings, content and schedules, fresh world.` : 'Paste a share code from a friend to start from their recipe.'}</span><button class="btn" onclick={() => (open = true)}><Icon name="package" size={15} />Use a blueprint</button></div>
    {:else}
      <div class="field"><label for="bps">Blueprint</label>
        <select id="bps" class="select" bind:value={id} onchange={() => { if (bp && !name) name = bp.name.slice(0, 48); }}>
          <option value="">Choose one…</option>
          {#each list as b (b.id)}<option value={b.id}>{b.name} ({b.runtime} {b.game_version})</option>{/each}
        </select>
        {#if bp}<span class="hint">{bp.properties} settings, {bp.content} content items, {bp.schedules} schedules. Needs {bp.memory_mb} MB.{#if bp.memory_mb > maxMemoryMb()} This machine has less memory than that, so creating it will be refused.{/if}</span>{/if}
      </div>
      <div class="field"><label for="bpc">Or paste a share code</label>
        <div class="row g6"><input id="bpc" class="input mono" style="flex:1" spellcheck="false" placeholder="moss1_…" bind:value={code} /><button class="btn" disabled={!code.trim()} onclick={useCode}>Use code</button></div>
        {#if codeErr}<span class="hint" style="color:var(--danger)" role="alert">{codeErr}</span>{:else}<span class="hint">The code is checked and saved as a blueprint here. Nothing runs until you create the server.</span>{/if}</div>
      <div class="field"><label for="bpn">Name for the new server</label><input id="bpn" class="input" maxlength="48" bind:value={name} /></div>
      <label class="row g6"><input type="checkbox" class="check" bind:checked={eula} /><span class="sm">I accept the <a href="https://aka.ms/MinecraftEULA" target="_blank" rel="noreferrer noopener" style="text-decoration:underline">Minecraft EULA</a>. This acceptance is recorded and audited. Optional: leave it unticked to accept later; the server won't start until then.</span></label>
      {#if err}<div class="banner danger" role="alert"><Icon name="alert" size={16} /><span>{err}</span></div>{/if}
      <div class="row end"><button class="btn" onclick={() => (open = false)}>Cancel</button><button class="btn primary" disabled={busy || !bp || !name.trim()} onclick={go}>{busy ? 'Creating…' : 'Create from blueprint'}</button></div>
    {/if}
  </section>
{/if}
