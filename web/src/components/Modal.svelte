<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  let { title, onclose, side = false, width = 520, children, footer }: {
    title: string; onclose: () => void; side?: boolean; width?: number; children: Snippet; footer?: Snippet;
  } = $props();
  let el: HTMLDivElement;
  const prev = document.activeElement as HTMLElement | null;
  const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])';

  $effect(() => {
    const first = el.querySelector<HTMLElement>('[data-autofocus]') ?? el.querySelector<HTMLElement>('.body ' + FOCUSABLE.split(',').join(',.body ')) ?? el;
    first.focus();
    document.body.style.overflow = 'hidden';
    return () => { document.body.style.overflow = ''; prev?.focus?.(); };
  });

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.stopPropagation(); onclose(); return; }
    if (e.key !== 'Tab') return;
    const f = [...el.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((x) => x.offsetParent !== null);
    if (!f.length) return;
    const a = f[0], z = f[f.length - 1];
    if (e.shiftKey && (document.activeElement === a || document.activeElement === el)) { e.preventDefault(); z.focus(); }
    else if (!e.shiftKey && document.activeElement === z) { e.preventDefault(); a.focus(); }
  }
</script>

<div class="overlay" class:side role="presentation" onmousedown={(e) => e.target === e.currentTarget && onclose()}>
  <div class="panel" class:side style="--w:{width}px" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" bind:this={el} onkeydown={key}>
    <header>
      <h2>{title}</h2>
      <button class="btn ghost icon sm" aria-label="Close" onclick={onclose}><Icon name="x" /></button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 60; background: var(--overlay); display: grid; place-items: center; padding: 16px; animation: fade .15s var(--ease); }
  .overlay.side { place-items: stretch end; padding: 0; }
  .panel {
    width: min(var(--w), 100%); max-height: min(86vh, 900px); display: flex; flex-direction: column; background: var(--s1); border: 1px solid var(--line2);
    border-radius: var(--r-lg); box-shadow: 0 24px 60px rgb(0 0 0 / .5), 0 1px 0 var(--hl) inset; animation: rise .18s var(--ease); outline: none;
  }
  .panel.side { max-height: none; height: 100%; border-radius: var(--r-lg) 0 0 var(--r-lg); border-right: 0; animation: slidein .2s var(--ease); }
  header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 16px 12px 20px; border-bottom: 1px solid var(--line); }
  .body { padding: 16px 20px; overflow: auto; flex: 1; min-height: 0; }
  footer { display: flex; gap: 8px; justify-content: flex-end; flex-wrap: wrap; padding: 12px 20px 16px; border-top: 1px solid var(--line); }
  @keyframes slidein { from { transform: translateX(24px); opacity: 0; } }
  @media (max-width: 600px) { .panel.side { width: 100%; border-radius: 0; } }
</style>
