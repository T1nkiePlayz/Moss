<script lang="ts">
  import AuthShell from './AuthShell.svelte';
  import Icon from '../../components/Icon.svelte';
  import { apiOps, ApiError } from '../../lib/api';
  import { auth, recheck, signOut } from '../../lib/auth.svelte';

  let token = $state('');
  let busy = $state(false);
  let error = $state('');
  let done = $state(false);
  let el: HTMLInputElement | undefined = $state();
  $effect(() => el?.focus());

  async function claim(e: Event) {
    e.preventDefault();
    if (!token.trim() || busy) return;
    busy = true; error = '';
    try {
      await apiOps.claimSetup(token.trim());
      done = true;
      setTimeout(() => void recheck(), 900); // let the success message be read
    } catch (er) {
      error = er instanceof ApiError && (er.status === 429 || er.code === 'auth.throttled')
        ? 'Too many attempts. Wait a minute before trying again.'
        : er instanceof Error && er.message ? er.message : 'That token was not accepted.';
    }
    busy = false;
  }
</script>

<AuthShell wide>
  <div>
    <h1>Set up this Moss instance</h1>
    <p class="muted sm">Nobody owns this instance yet. Prove you control the machine with the one-time setup token and your account <b>{auth.email || 'you are signed in with'}</b> becomes the owner.</p>
  </div>

  <div class="banner info">
    <Icon name="key" size={16} />
    <div>
      <b>Where is the token?</b>
      <div class="muted">Moss wrote it the first time it started, to the file <code>setup-token</code> inside its data directory (<code>&lt;data dir&gt;/setup-token</code>), and logged where that file is. Read the file on the host or search the service log for <code>setup</code>. It is deleted once the instance is claimed.</div>
    </div>
  </div>

  <form class="col" style="gap:14px" onsubmit={claim}>
    <div class="field">
      <label for="setup-token">Setup token</label>
      <input id="setup-token" class="input mono" bind:value={token} required autocomplete="off" autocapitalize="off" spellcheck="false" aria-describedby="setup-hint setup-err" aria-invalid={error ? 'true' : undefined} disabled={done} bind:this={el} />
      <span class="hint" id="setup-hint">Paste it exactly as written. You have 5 attempts per minute.</span>
    </div>
    <div id="setup-err" aria-live="assertive">{#if error}<div class="banner danger" role="alert"><Icon name="alert" size={16} />{error}</div>{/if}</div>
    <div aria-live="polite">{#if done}<div class="banner info" role="status"><Icon name="check-circle" size={16} />Instance claimed. You are the owner. Opening Moss…</div>{/if}</div>
    <button class="btn primary lg" disabled={busy || done || !token.trim()}>{busy ? 'Checking…' : 'Claim this instance'}</button>
  </form>
  <button class="btn ghost" onclick={signOut}><Icon name="logout" size={15} />Sign out</button>
</AuthShell>
