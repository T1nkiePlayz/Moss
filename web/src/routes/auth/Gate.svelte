<script lang="ts">
  import Setup from './Setup.svelte';
  import NotInvited from './NotInvited.svelte';
  import Mfa from './Mfa.svelte';
  import Throttled from './Throttled.svelte';
  import { auth } from '../../lib/auth.svelte';
</script>

{#if auth.phase === 'setup'}<Setup />
{:else if auth.phase === 'denied'}<NotInvited />
{:else if auth.phase === 'mfa'}<Mfa />
{:else if auth.phase === 'throttled'}<Throttled />{/if}
