<script lang="ts">
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import { auth, signIn, signUp, oauth } from '../lib/auth.svelte';
  let mode = $state<'in' | 'up'>('in');
  let email = $state('');
  let pw = $state('');
  function submit(e: Event) { e.preventDefault(); void (mode === 'in' ? signIn(email, pw) : signUp(email, pw)); }
</script>

<div class="wrap">
  <div class="glow"></div>
  <form class="card box rise" onsubmit={submit}>
    <Logo size={40} />
    <div><h1>{mode === 'in' ? 'Welcome back' : 'Create your account'}</h1><p class="muted sm">{mode === 'in' ? 'Sign in to control your servers.' : 'The owner must invite your email before you can use Moss.'}</p></div>
    <div class="row">
      <button type="button" class="btn grow" onclick={() => oauth('github')}><Icon name="github" size={16} />GitHub</button>
      <button type="button" class="btn grow" onclick={() => oauth('google')}><Icon name="google" size={16} />Google</button>
    </div>
    <div class="or xs faint"><span>or with email</span></div>
    <div class="field"><label for="em">Email</label><input id="em" class="input" type="email" autocomplete="email" required bind:value={email} /></div>
    <div class="field"><label for="pw">Password</label><input id="pw" class="input" type="password" autocomplete={mode === 'in' ? 'current-password' : 'new-password'} required minlength="6" bind:value={pw} /></div>
    {#if auth.config?.setup_required}<div class="banner info" role="status"><Icon name="key" size={16} /><span>This instance has no owner yet. Sign in, then enter the one-time setup token from the host to claim it.</span></div>{/if}
    {#if auth.config?.access_message}<p class="xs muted access">{auth.config.access_message}</p>{/if}
    {#if auth.error}<div class="banner danger" role="alert">{auth.error}</div>{/if}
    {#if auth.notice}<div class="banner info" role="status">{auth.notice}</div>{/if}
    <button class="btn primary lg" disabled={auth.busy}>{auth.busy ? 'Please wait…' : mode === 'in' ? 'Sign in' : 'Sign up'}</button>
    <button type="button" class="btn ghost" onclick={() => { mode = mode === 'in' ? 'up' : 'in'; auth.error = ''; auth.notice = ''; }}>
      {mode === 'in' ? 'New here? Create an account' : 'Have an account? Sign in'}
    </button>
  </form>
</div>

<style>
  .wrap { min-height: 100vh; display: grid; place-items: center; padding: 20px; position: relative; isolation: isolate; }
  .glow { position: absolute; inset: 0; z-index: -1; background: radial-gradient(600px 360px at 50% 30%, var(--a-soft), transparent 70%); }
  .box { width: min(400px, 100%); display: flex; flex-direction: column; gap: 16px; padding: 28px; }
  .access { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .or { display: flex; align-items: center; gap: 10px; } .or::before, .or::after { content: ''; height: 1px; flex: 1; background: var(--line); }
</style>
