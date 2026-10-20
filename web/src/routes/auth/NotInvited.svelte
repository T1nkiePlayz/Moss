<script lang="ts">
  import AuthShell from './AuthShell.svelte';
  import Icon from '../../components/Icon.svelte';
  import { auth, recheck, signOut } from '../../lib/auth.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  let busy = $state(false);
  async function again() { busy = true; await recheck(); busy = false; }
  const msg = $derived(auth.config?.access_message?.trim());
</script>

<AuthShell>
  <div role="alert">
    <h1>You're not invited yet</h1>
    <p class="muted sm">You are signed in, but this Moss instance has no account for you. Only people the owner invites can use it.</p>
  </div>

  <div class="field">
    <span class="label" id="ni-mail">Signed in as</span>
    <div class="row g6">
      <code class="grow ellipsis" aria-labelledby="ni-mail">{auth.email || 'unknown address'}</code>
      {#if auth.email}<button class="btn sm" onclick={() => copyWithToast(auth.email, 'Email')} aria-label="Copy email address"><Icon name="copy" size={13} />Copy</button>{/if}
    </div>
  </div>

  {#if msg}
    <figure class="msg"><figcaption class="xs muted">Message from the owner</figcaption><blockquote>{msg}</blockquote></figure>
  {/if}
  <p class="sm">Ask the owner to <b>invite this email address</b> (Users, then Add user), then check again.</p>

  <div class="row wrap">
    <button class="btn primary" onclick={again} disabled={busy}><Icon name="refresh" size={15} />{busy ? 'Checking…' : 'Check again'}</button>
    <button class="btn" onclick={signOut}><Icon name="logout" size={15} />Sign out</button>
  </div>
</AuthShell>

<style>
  .msg { margin: 0; padding: 12px 14px; border-radius: var(--r-sm); background: var(--info-soft); border: 1px solid color-mix(in srgb, var(--info), transparent 70%); }
  blockquote { margin: 4px 0 0; white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
