<script lang="ts">
  import Icon from './Icon.svelte';
  import { toasts, dismissToast } from '../lib/stores.svelte';
  const icon = { info: 'info', success: 'check-circle', warn: 'alert', error: 'x-circle' } as const;
</script>

<div class="toasts" aria-live="polite" aria-atomic="false" role="region" aria-label="Notifications">
  {#each toasts.list as t (t.id)}
    <div class="toast {t.kind}" role={t.kind === 'error' ? 'alert' : 'status'}>
      <span class="ic"><Icon name={icon[t.kind]} size={18} /></span>
      <div class="grow">
        <div class="title">{t.title}</div>
        {#if t.body}<div class="muted sm">{t.body}</div>{/if}
      </div>
      {#if t.action}
        <button class="btn sm" onclick={() => { t.action!.run(); dismissToast(t.id); }}>{#if t.action.label === 'Undo'}<Icon name="undo" size={14} />{/if}{t.action.label}</button>
      {/if}
      <button class="btn ghost icon sm" aria-label="Dismiss" onclick={() => dismissToast(t.id)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts { position: fixed; z-index: 80; right: 16px; bottom: 16px; display: flex; flex-direction: column; gap: 8px; width: min(380px, calc(100vw - 32px)); pointer-events: none; }
  .toast { pointer-events: auto; display: flex; gap: 10px; align-items: center; padding: 10px 10px 10px 14px; border-radius: var(--r); background: var(--s3); border: 1px solid var(--line2); box-shadow: 0 12px 32px rgb(0 0 0 / .4), 0 1px 0 var(--hl) inset; animation: rise .2s var(--ease); }
  .title { font-weight: 600; font-size: var(--fs-sm); }
  .ic { display: grid; color: var(--info); } .success .ic { color: var(--a-text); } .warn .ic { color: var(--warn); } .error .ic { color: var(--danger); }
  .toast.error { border-color: color-mix(in srgb, var(--danger), transparent 55%); }
  @media (max-width: 760px) { .toasts { bottom: 76px; } }
</style>
