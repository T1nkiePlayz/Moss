<script lang="ts">
  import Icon from './Icon.svelte';
  import States from './States.svelte';
  import { inbox, markRead, refreshNotifications, serverById } from '../lib/stores.svelte';
  import { groupNotifications, severityIcon, severityLabel, kindLabel } from '../lib/notify';
  import { relTime, dateTime } from '../lib/format';

  let { onclose }: { onclose: () => void } = $props();
  const groups = $derived(groupNotifications(inbox.items));
  let first: HTMLElement | undefined = $state();
  $effect(() => { first?.focus(); });
  $effect(() => { void refreshNotifications(); }); // opening the inbox always shows fresh data
  const open = (id: string, read: boolean) => { if (!read) void markRead([id]); onclose(); };
</script>

<div class="head">
  <h2>Notifications</h2>
  <span class="grow"></span>
  <button class="btn sm" bind:this={first} disabled={!inbox.unread} onclick={() => markRead()}><Icon name="check" size={13} />Mark all read</button>
</div>
<div class="list" aria-live="polite">
  {#if inbox.error && !inbox.items.length}
    <div class="pad"><States kind="error" text={inbox.error} retry={refreshNotifications} /></div>
  {:else if !inbox.loaded}
    <div class="pad"><States kind="loading" rows={3} /></div>
  {:else if !inbox.items.length}
    <div class="pad"><States kind="empty" icon="bell" title="You're all caught up" text="Crashes, failed backups and low disk space will show up here." /></div>
  {:else}
    {#each groups as g (g.label)}
      <h3 class="day">{g.label}</h3>
      <ul>
        {#each g.items as n (n.id)}
          {@const srv = n.server_id ? serverById(n.server_id) : undefined}
          <li class="n {n.severity}" class:unread={!n.read}>
            <span class="ic" title={severityLabel[n.severity]}><Icon name={severityIcon[n.severity]} size={17} label={severityLabel[n.severity]} /></span>
            <div class="grow">
              <div class="t">{n.title}{#if !n.read}<span class="dot-u" role="img" aria-label="Unread"></span>{/if}</div>
              {#if n.body}<div class="b sm muted">{n.body}</div>{/if}
              <div class="meta xs faint">
                <span class="tag">{kindLabel[n.kind] ?? n.kind}</span>
                <time datetime={n.time} title={dateTime(n.time)}>{relTime(n.time)}</time>
                {#if n.server_id}<a href="#/servers/{encodeURIComponent(n.server_id)}" onclick={() => open(n.id, n.read)}>{srv?.name ?? 'Open server'}</a>{/if}
                {#if !n.read}<button class="link" onclick={() => markRead([n.id])}>Mark read</button>{/if}
              </div>
            </div>
          </li>
        {/each}
      </ul>
    {/each}
  {/if}
</div>

<style>
  .head { display: flex; align-items: center; gap: 8px; padding: 12px 14px; border-bottom: 1px solid var(--line); }
  h2 { font-size: var(--fs-md); }
  .list { overflow: auto; flex: 1; min-height: 120px; }
  .pad { padding: 14px; }
  .day { font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: .06em; color: var(--faint); font-weight: 600; padding: 10px 14px 4px; }
  ul { list-style: none; margin: 0; padding: 0; }
  .n { display: flex; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--line); border-left: 3px solid transparent; }
  .n.unread { background: var(--hl); }
  .n.critical { border-left-color: var(--danger); } .n.warn { border-left-color: var(--warn); } .n.info { border-left-color: var(--info); }
  .ic { display: grid; margin-top: 1px; color: var(--info); } .critical .ic { color: var(--danger); } .warn .ic { color: var(--warn); }
  .t { font-weight: 600; font-size: var(--fs-sm); display: flex; align-items: center; gap: 6px; }
  .dot-u { width: 7px; height: 7px; border-radius: 50%; background: var(--a1); flex: none; }
  .b { margin-top: 2px; overflow-wrap: anywhere; }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; margin-top: 6px; }
  .link { background: none; border: 0; padding: 0; color: var(--a-text); cursor: pointer; font: inherit; } .link:hover { text-decoration: underline; }
</style>
