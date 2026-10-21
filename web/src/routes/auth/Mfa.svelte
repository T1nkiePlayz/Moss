<script lang="ts">
  import AuthShell from './AuthShell.svelte';
  import Icon from '../../components/Icon.svelte';
  import { auth, supabaseCall, adoptSession, recheck, signOut } from '../../lib/auth.svelte';
  import { encodeQr, qrPath } from '../../lib/qr';
  import { otpauthUri, groupSecret, normalizeCode, isCode } from '../../lib/totp';
  import { copyWithToast } from '../../lib/clipboard';

  type Factor = { id: string; factor_type: string; status: string };
  let mode = $state<'loading' | 'enrol' | 'verify' | 'error'>('loading');
  let factorId = '';
  let secret = $state('');
  let uri = $state('');
  let qr = $state<{ d: string; size: number } | null>(null);
  let code = $state('');
  let busy = $state(false);
  let error = $state('');
  let input: HTMLInputElement | undefined = $state();

  const friendly = (e: unknown) => {
    const x = e as { code?: string; message?: string };
    if (x.code === 'mfa_verification_failed' || /invalid totp/i.test(x.message ?? '')) return 'That code did not match. Codes change every 30 seconds, so wait for a fresh one and try again.';
    if (x.code === 'mfa_challenge_expired') return 'The challenge expired. Enter a fresh code.';
    return x.message || 'Something went wrong.';
  };

  async function enrol() {
    const r = await supabaseCall('POST', 'factors', { factor_type: 'totp', friendly_name: `Moss ${Date.now().toString(36)}`, issuer: 'Moss' });
    factorId = r.id;
    secret = r.totp?.secret ?? '';
    uri = r.totp?.uri || otpauthUri({ issuer: 'Moss', account: auth.email || 'moss', secret });
    try { qr = qrPath(encodeQr(uri)); } catch { qr = null; }
    mode = 'enrol';
  }
  async function init() {
    mode = 'loading'; error = '';
    try {
      const u = await supabaseCall('GET', 'user');
      const totp: Factor[] = (u.factors ?? []).filter((f: Factor) => f.factor_type === 'totp');
      const ok = totp.find((f) => f.status === 'verified');
      if (ok) { factorId = ok.id; mode = 'verify'; return; }
      for (const f of totp) await supabaseCall('DELETE', `factors/${f.id}`).catch(() => {}); // abandoned enrolments
      await enrol();
    } catch (e) { error = friendly(e); mode = 'error'; }
  }
  $effect(() => { void init(); });
  $effect(() => { if (mode === 'enrol' || mode === 'verify') input?.focus(); });

  async function submit(e: Event) {
    e.preventDefault();
    if (!isCode(code) || busy) return;
    busy = true; error = '';
    try {
      const ch = await supabaseCall('POST', `factors/${factorId}/challenge`, {});
      adoptSession(await supabaseCall('POST', `factors/${factorId}/verify`, { challenge_id: ch.id, code }));
      await recheck();
      if (auth.phase === 'mfa') { error = 'The code was accepted but Moss still sees a single-factor session. Sign out and sign in again.'; code = ''; }
    } catch (er) { error = friendly(er); code = ''; input?.focus(); }
    busy = false;
  }
</script>

<AuthShell wide>
  <div>
    <h1>{mode === 'enrol' ? 'Set up two-step verification' : 'Two-step verification'}</h1>
    <p class="muted sm">{#if mode === 'enrol'}This Moss instance requires a second factor. Add Moss to an authenticator app such as 1Password, Aegis or Google Authenticator.{:else}This Moss instance requires a second factor. Enter the current code from your authenticator app.{/if}</p>
  </div>

  {#if mode === 'loading'}
    <div class="skel" style="height:120px" aria-busy="true" aria-label="Loading"></div>
  {:else if mode === 'error'}
    <div class="banner danger" role="alert"><Icon name="alert" size={16} />{error}</div>
    <button class="btn" onclick={init}><Icon name="refresh" size={15} />Retry</button>
  {:else}
    {#if mode === 'enrol'}
      <ol class="steps">
        <li>
          <b>Scan this QR code</b>
          {#if qr}
            <svg class="qr" viewBox="0 0 {qr.size} {qr.size}" role="img" aria-label="QR code for the authenticator app" shape-rendering="crispEdges"><rect width={qr.size} height={qr.size} fill="#fff" /><path d={qr.d} fill="#000" /></svg>
          {:else}<p class="muted sm">The QR code could not be drawn here. Use the key below instead.</p>{/if}
        </li>
        <li>
          <b>Or enter the key by hand</b>
          <div class="row g6 wrap">
            <code class="secret" aria-label="Secret key">{groupSecret(secret)}</code>
            <button type="button" class="btn sm" onclick={() => copyWithToast(secret, 'Secret key')}><Icon name="copy" size={13} />Copy key</button>
            <button type="button" class="btn sm" onclick={() => copyWithToast(uri, 'Setup link')}><Icon name="link" size={13} />Copy link</button>
          </div>
        </li>
        <li><b>Enter the 6-digit code it shows</b></li>
      </ol>
    {/if}
    <form class="col" style="gap:12px" onsubmit={submit}>
      <div class="field">
        <label for="otp">{mode === 'enrol' ? 'Verification code' : 'Authenticator code'}</label>
        <input id="otp" class="input otp mono" bind:this={input} value={code} oninput={(e) => { code = normalizeCode(e.currentTarget.value); e.currentTarget.value = code; }}
          inputmode="numeric" pattern="[0-9]*" maxlength="7" autocomplete="one-time-code" placeholder="123456" required aria-describedby="otp-err" aria-invalid={error ? 'true' : undefined} />
      </div>
      <div id="otp-err" aria-live="assertive">{#if error}<div class="banner danger" role="alert"><Icon name="alert" size={16} />{error}</div>{/if}</div>
      <button class="btn primary lg" disabled={busy || !isCode(code)}>{busy ? 'Verifying…' : mode === 'enrol' ? 'Verify and continue' : 'Verify'}</button>
    </form>
  {/if}
  <button class="btn ghost" onclick={signOut}><Icon name="logout" size={15} />Sign out</button>
</AuthShell>

<style>
  .steps { margin: 0; padding: 0; list-style: none; counter-reset: s; display: flex; flex-direction: column; gap: 14px; }
  .steps li { counter-increment: s; position: relative; padding-left: 32px; display: flex; flex-direction: column; gap: 8px; }
  .steps li::before { content: counter(s); position: absolute; left: 0; top: 0; width: 22px; height: 22px; border-radius: 50%; display: grid; place-items: center; font-size: var(--fs-xs); font-weight: 700; background: var(--a-soft); color: var(--a-text); }
  .qr { width: 188px; height: 188px; border-radius: 10px; border: 1px solid var(--line2); }
  .secret { font-size: var(--fs-sm); letter-spacing: .04em; overflow-wrap: anywhere; }
  .otp { font-size: calc(22px * var(--ts, 1)); letter-spacing: .3em; text-align: center; height: 48px; }
</style>
