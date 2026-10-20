<script lang="ts">
  import Logo from './components/Logo.svelte';
  import Lazy from './components/Lazy.svelte';
  import Toasts from './components/Toasts.svelte';
  import States from './components/States.svelte';
  import { auth, boot } from './lib/auth.svelte';
  import { applyPrefs, dialog } from './lib/stores.svelte';

  applyPrefs();
  void boot();

  // Each screen is its own chunk: the first paint needs only the loading mark, then exactly one of these.
  const login = () => import('./components/Login.svelte');
  const gate = () => import('./routes/auth/Gate.svelte');
  const shell = () => import('./Shell.svelte');
  // Most visits are signed in already: fetch the app shell while /me is answering so it is not a second round trip.
  $effect(() => { if (auth.phase === 'loading' && auth.config && (auth.config.mode === 'dev' || localStorage.getItem('moss.session'))) void shell(); });
  const confirmHost = () => import('./components/ConfirmHost.svelte');
</script>

{#if auth.phase === 'loading'}
  <div class="boot"><Logo size={44} /><div class="bar indet" style="width:120px"><i></i></div></div>
{:else if auth.phase === 'login'}
  <Lazy load={login} />
{:else if auth.phase === 'denied' || auth.phase === 'setup' || auth.phase === 'mfa' || auth.phase === 'throttled'}
  <Lazy load={gate} />
{:else if auth.phase === 'error'}
  <div class="boot"><States kind="error" title="Cannot reach Moss" text={auth.error || 'The backend did not respond.'} retry={() => boot()} /></div>
{:else}
  <Lazy load={shell} />
{/if}
{#if dialog.req}<Lazy load={confirmHost} />{/if}
<Toasts />

<style>
  .boot { min-height: 100vh; display: grid; place-content: center; justify-items: center; gap: 20px; padding: 20px; }
</style>
