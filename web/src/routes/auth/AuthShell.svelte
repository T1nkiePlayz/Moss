<script lang="ts">
  import type { Snippet } from 'svelte';
  import Logo from '../../components/Logo.svelte';
  let { wide = false, children }: { wide?: boolean; children: Snippet } = $props();
</script>

<main class="wrap" id="main">
  <div class="glow"></div>
  <div class="card box rise" class:wide>
    <Logo size={40} />
    {@render children()}
  </div>
</main>

<style>
  .wrap { min-height: 100vh; display: grid; place-items: center; padding: 20px; position: relative; isolation: isolate; }
  .glow { position: absolute; inset: 0; z-index: -1; background: radial-gradient(600px 360px at 50% 30%, var(--a-soft), transparent 70%); }
  .box { width: min(420px, 100%); display: flex; flex-direction: column; gap: 16px; padding: 28px; }
  .box.wide { width: min(520px, 100%); }
  @media (max-width: 420px) { .box { padding: 20px; } }
</style>
