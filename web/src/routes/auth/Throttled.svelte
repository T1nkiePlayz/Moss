<script lang="ts">
  import AuthShell from './AuthShell.svelte';
  import Icon from '../../components/Icon.svelte';
  import { auth, boot, signOut } from '../../lib/auth.svelte';
  let wait = $state(15);
  $effect(() => {
    const t = setInterval(() => { wait = Math.max(0, wait - 1); }, 1000);
    return () => clearInterval(t);
  });
</script>

<AuthShell>
  <div role="alert">
    <h1>Slow down a little</h1>
    <p class="muted sm">Moss paused requests from your address after too many sign-in attempts in a short time. This protects the instance from password guessing and clears by itself within a minute.</p>
  </div>
  {#if auth.error}<p class="xs faint">{auth.error}</p>{/if}
  <div class="row wrap">
    <button class="btn primary" disabled={wait > 0} onclick={() => boot()}><Icon name="refresh" size={15} />{wait > 0 ? `Try again in ${wait}s` : 'Try again'}</button>
    <button class="btn" onclick={signOut}><Icon name="logout" size={15} />Sign out</button>
  </div>
</AuthShell>
