<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import AnsiLine from '../../components/AnsiLine.svelte';
  import { api, openConsole, saveBlob, errMsg, type ConsoleSocket, type Server } from '../../lib/api';
  import { levelOf, stripAnsi, type Level } from '../../lib/ansi';
  import { toastError } from '../../lib/stores.svelte';
  import { can } from '../../lib/auth.svelte';
  import { tick } from 'svelte';

  let { server: s }: { server: Server } = $props();
  const CAP = 5000, ROW = 20, OVER = 20;

  let lines = $state.raw<string[]>([]);
  let conn = $state<'connecting' | 'live' | 'down'>('connecting');
  let loadErr = $state('');
  let q = $state('');
  let lv = $state<Record<Level, boolean>>({ info: true, warn: true, error: true, debug: true, chat: true });
  let stick = $state(true);
  let unseen = $state(0);
  let vp: HTMLDivElement;
  let scrollTop = $state(0);
  let vh = $state(400);
  let cmd = $state('');
  let hist: string[] = [];
  let hi = -1;
  try { hist = JSON.parse(localStorage.getItem('moss.cmdhist') || '[]'); } catch { /* ignore */ }

  const canWrite = $derived(can('server.console.write'));
  const needle = $derived(q.trim().toLowerCase());
  const filtered = $derived.by(() => {
    const all = Object.values(lv).every(Boolean);
    if (all && !needle) return lines;
    return lines.filter((l) => (all || lv[levelOf(l)]) && (!needle || stripAnsi(l).toLowerCase().includes(needle)));
  });
  const start = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVER));
  const end = $derived(Math.min(filtered.length, Math.ceil((scrollTop + vh) / ROW) + OVER));
  const slice = $derived(filtered.slice(start, end));

  // Batch socket lines per animation frame.
  let pending: string[] = [];
  let raf = 0;
  function push(l: string) {
    pending.push(...l.split('\n').filter((x, _i, a) => x !== '' || a.length === 1));
    raf ||= requestAnimationFrame(flush);
  }
  async function flush() {
    raf = 0;
    const add = pending; pending = [];
    if (!add.length) return;
    const next = lines.concat(add);
    lines = next.length > CAP ? next.slice(-CAP) : next;
    if (stick) { await tick(); toBottom(); } else unseen += add.length;
  }
  function toBottom() { if (vp) { vp.scrollTop = vp.scrollHeight; scrollTop = vp.scrollTop; } }
  function onScroll() {
    scrollTop = vp.scrollTop;
    stick = vp.scrollHeight - vp.scrollTop - vp.clientHeight < 40;
    if (stick) unseen = 0;
  }
  function jump() { stick = true; unseen = 0; toBottom(); }

  $effect(() => {
    const id = s.id;
    let sock: ConsoleSocket | null = null;
    let dead = false;
    let tries = 0;
    let timer: ReturnType<typeof setTimeout>;
    lines = []; loadErr = ''; stick = true; unseen = 0;
    api.backlog(id, 500).then(async (b) => { lines = b.lines.concat(lines); await tick(); toBottom(); }).catch((e) => (loadErr = errMsg(e)));
    const connect = () => {
      conn = 'connecting';
      sock = openConsole(id, {
        onOpen: () => { conn = 'live'; tries = 0; sockRef = sock; },
        onClose: () => { if (dead) return; conn = 'down'; sockRef = null; timer = setTimeout(connect, Math.min(15000, 1000 * 2 ** tries++)); },
        onMsg: (m) => { if (m.t === 'line') push(m.d); else if (m.t === 'state') push(`\x1b[2m— server state: ${m.d} —\x1b[0m`); },
      });
      sockRef = sock;
    };
    connect();
    return () => { dead = true; clearTimeout(timer); sock?.close(); sockRef = null; cancelAnimationFrame(raf); raf = 0; };
  });
  let sockRef: ConsoleSocket | null = null;

  async function send(e: Event) {
    e.preventDefault();
    const c = cmd.trim();
    if (!c) return;
    cmd = ''; hi = -1;
    hist = [c, ...hist.filter((x) => x !== c)].slice(0, 50);
    try { localStorage.setItem('moss.cmdhist', JSON.stringify(hist)); } catch { /* ignore */ }
    try {
      if (conn === 'live' && sockRef) sockRef.send(c); else await api.command(s.id, c);
    } catch (err) { toastError(err, 'Command failed'); }
  }
  function histKey(e: KeyboardEvent) {
    if (e.key === 'ArrowUp' && hist.length) { e.preventDefault(); hi = Math.min(hist.length - 1, hi + 1); cmd = hist[hi]; }
    else if (e.key === 'ArrowDown') { e.preventDefault(); hi = Math.max(-1, hi - 1); cmd = hi < 0 ? '' : hist[hi]; }
  }
  function download() {
    saveBlob(new Blob([filtered.map(stripAnsi).join('\n') + '\n'], { type: 'text/plain' }), `${s.name.replace(/\W+/g, '-')}-console.log`);
  }
  const levels: [Level, string][] = [['info', 'Info'], ['warn', 'Warn'], ['error', 'Error'], ['debug', 'Debug']];
</script>

<div class="cwrap">
  <div class="bar-top row wrap">
    <span class="badge {conn === 'live' ? 'ok' : conn === 'down' ? 'danger' : 'warn'}" role="status"><span class="dot {conn === 'live' ? 'running' : conn === 'down' ? 'crashed' : 'starting'}"></span>{conn === 'live' ? 'Live' : conn === 'down' ? 'Disconnected · retrying' : 'Connecting'}</span>
    <div class="grow" style="max-width:280px;position:relative;min-width:140px">
      <input class="input sm" style="padding-left:30px" type="search" placeholder="Filter lines…" aria-label="Filter console lines" bind:value={q} />
      <span class="ico"><Icon name="search" size={14} /></span>
    </div>
    <div class="row g4 wrap" role="group" aria-label="Log levels">
      {#each levels as [k, l]}<button class="chip" style="height:28px;padding:0 10px" aria-pressed={lv[k]} onclick={() => (lv[k] = !lv[k])}>{l}</button>{/each}
    </div>
    <div class="grow"></div>
    <span class="xs faint tnum">{filtered.length === lines.length ? lines.length : `${filtered.length} / ${lines.length}`} lines</span>
    <button class="btn sm" onclick={download} aria-label="Download visible lines"><Icon name="download" size={14} />Download</button>
    <button class="btn sm ghost" onclick={() => (lines = [])}>Clear</button>
  </div>

  <div class="term" role="log" aria-label="Server console" aria-live="off">
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="vp mono" bind:this={vp} bind:clientHeight={vh} onscroll={onScroll} tabindex="0">
      {#if loadErr && !lines.length}
        <div class="msg">Could not load backlog: {loadErr}</div>
      {:else if !filtered.length}
        <div class="msg faint">{lines.length ? 'No lines match the filter.' : conn === 'live' ? 'Waiting for output…' : 'No output yet.'}</div>
      {:else}
        <div style="height:{filtered.length * ROW}px;position:relative;min-width:max-content">
          <div class="win" style="top:{start * ROW}px">
            {#each slice as l, i (start + i)}
              <div class="ln {levelOf(l)}"><AnsiLine line={l} /></div>
            {/each}
          </div>
        </div>
      {/if}
    </div>
    {#if !stick}
      <button class="jump btn sm primary" onclick={jump}><Icon name="chevron-down" size={14} />Jump to latest{#if unseen} · {unseen} new{/if}</button>
    {/if}
  </div>

  <form class="cmd row" onsubmit={send}>
    <span class="prompt mono" aria-hidden="true">&gt;</span>
    <input class="input mono" style="border:0;background:transparent;box-shadow:none" placeholder={canWrite ? 'Type a command… (↑ ↓ for history)' : 'You do not have permission to send commands'} aria-label="Console command" autocomplete="off" spellcheck="false" bind:value={cmd} onkeydown={histKey} disabled={!canWrite || (s.state !== 'running' && s.state !== 'starting')} />
    <button class="btn primary sm" disabled={!cmd.trim() || !canWrite}>Send</button>
  </form>
  {#if s.state !== 'running' && s.state !== 'starting'}<p class="hint" style="margin-top:6px">The server is {s.state}. Start it to send commands.</p>{/if}
</div>

<style>
  .cwrap { display: flex; flex-direction: column; gap: 10px; height: calc(100dvh - 250px); min-height: 380px; }
  .ico { position: absolute; left: 9px; top: 8px; color: var(--faint); pointer-events: none; display: grid; }
  .term { position: relative; flex: 1; min-height: 0; background: #060908; border: 1px solid var(--line); border-radius: var(--r); box-shadow: inset 0 1px 0 var(--hl); overflow: hidden; color: #d7e3db; }
  :root[data-theme='light'] .term { background: #0e1511; }
  .vp { position: absolute; inset: 0; overflow: auto; font-size: 12.5px; padding: 8px 0; outline: none; overscroll-behavior: contain; }
  .vp:focus-visible { box-shadow: inset 0 0 0 2px var(--a1); }
  .win { position: absolute; left: 0; right: 0; }
  .ln { height: 20px; line-height: 20px; white-space: pre; padding: 0 14px; }
  .ln:hover { background: rgb(255 255 255 / .04); }
  .ln.warn { color: #f2b04c; background: rgb(242 176 76 / .05); } .ln.error { color: #ff7d71; background: rgb(255 107 94 / .08); }
  .ln.debug { color: #6b7d72; } .ln.chat { color: #b9f2a0; }
  .msg { padding: 24px; text-align: center; color: var(--muted); }
  .jump { position: absolute; bottom: 12px; left: 50%; transform: translateX(-50%); box-shadow: 0 8px 24px rgb(0 0 0 / .5); }
  .cmd { background: var(--s2); border: 1px solid var(--line2); border-radius: var(--r); padding: 4px 6px 4px 14px; }
  .cmd:focus-within { border-color: var(--a1); box-shadow: 0 0 0 3px var(--a-soft); }
  .prompt { color: var(--a-text); font-weight: 700; }
  @media (max-width: 860px) { .cwrap { height: calc(100dvh - 300px); } }
</style>
