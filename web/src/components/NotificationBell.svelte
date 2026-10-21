<script lang="ts">
  import Icon from './Icon.svelte';
  import Lazy from './Lazy.svelte';
  import { inbox } from '../lib/stores.svelte';

  const loadInbox = () => import('./NotificationInbox.svelte');
  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let btn: HTMLButtonElement | undefined = $state();
  const crit = $derived(inbox.items.some((n) => !n.read && n.severity === 'critical'));
  const label = $derived(`Notifications${inbox.unread ? `, ${inbox.unread} unread` : ''}`);

  function close(refocus = true) { open = false; if (refocus) btn?.focus(); }
  $effect(() => {
    if (!open) return;
    const off = (e: MouseEvent) => { if (root && !root.contains(e.target as Node)) close(false); };
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); close(); } };
    document.addEventListener('mousedown', off);
    document.addEventListener('keydown', esc);
    return () => { document.removeEventListener('mousedown', off); document.removeEventListener('keydown', esc); };
  });
</script>

<div class="bell" bind:this={root}>
  <button class="btn ghost icon" bind:this={btn} onclick={() => (open = !open)} aria-haspopup="dialog" aria-expanded={open} aria-controls="notif-pop" aria-label={label}>
    <Icon name="bell" />{#if inbox.unread}<span class="badge-n" class:crit aria-hidden="true">{inbox.unread > 99 ? '99+' : inbox.unread}</span>{/if}
  </button>
  {#if open}
    <div class="pop" id="notif-pop" role="dialog" aria-label="Notifications">
      <Lazy load={loadInbox} props={{ onclose: () => close(false) }} />
    </div>
  {/if}
</div>

<style>
  .bell { position: relative; }
  .badge-n { position: absolute; top: 1px; right: 1px; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--a1); color: var(--a-ink); font-size: calc(10px * var(--ts, 1)); font-weight: 700; display: grid; place-items: center; box-shadow: 0 0 0 2px var(--bg); }
  .badge-n.crit { background: var(--danger); color: #1a0402; }
  .pop { position: absolute; right: 0; top: 40px; width: min(400px, calc(100vw - 24px)); max-height: min(70vh, 560px); display: flex; flex-direction: column; padding: 0; background: var(--s2); border: 1px solid var(--line2); border-radius: var(--r); box-shadow: 0 16px 40px rgb(0 0 0 / .45); animation: rise .14s var(--ease); z-index: 6; overflow: hidden; }
  @media (max-width: 860px) { .pop { position: fixed; left: 12px; right: 12px; top: calc(var(--top) + 4px); width: auto; } }
</style>
