<script lang="ts">
  import Icon from './Icon.svelte';
  import type { Snippet } from 'svelte';
  // One component for the three non-happy states of any view.
  let { kind, title, text, icon = 'cube', retry, children, rows = 3 }: {
    kind: 'empty' | 'error' | 'loading'; title?: string; text?: string; icon?: string; retry?: () => void; children?: Snippet; rows?: number;
  } = $props();
</script>

{#if kind === 'loading'}
  <div class="col" aria-busy="true" aria-label="Loading">
    {#each Array(rows) as _}<div class="skel" style="height:52px"></div>{/each}
  </div>
{:else}
  <div class="state" class:error={kind === 'error'} role={kind === 'error' ? 'alert' : undefined}>
    <div class="ic"><Icon name={kind === 'error' ? 'alert' : icon} size={26} /></div>
    <h3>{title ?? (kind === 'error' ? 'Something went wrong' : 'Nothing here yet')}</h3>
    {#if text}<p class="muted">{text}</p>{/if}
    <div class="row wrap" style="justify-content:center;margin-top:6px">
      {#if kind === 'error' && retry}<button class="btn" onclick={retry}><Icon name="refresh" size={15} />Retry</button>{/if}
      {@render children?.()}
    </div>
  </div>
{/if}

<style>
  .state { display: flex; flex-direction: column; align-items: center; text-align: center; gap: 6px; padding: 40px 20px; border: 1px dashed var(--line2); border-radius: var(--r); background: color-mix(in srgb, var(--s1), transparent 40%); }
  .ic { width: 52px; height: 52px; display: grid; place-items: center; border-radius: 14px; background: var(--a-soft); color: var(--a-text); margin-bottom: 6px; }
  .error .ic { background: var(--danger-soft); color: var(--danger); }
  p { max-width: 44ch; }
</style>
