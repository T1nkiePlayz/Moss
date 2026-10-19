<script lang="ts">
  let { size = 28 }: { size?: number } = $props();
  const id = 'mg' + Math.random().toString(36).slice(2, 7); // ids must be unique per instance (hidden copies break url() refs)
</script>

<!-- leaf-in-cube mark -->
<svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
  <defs>
    <linearGradient id={id} x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="var(--a1)" /><stop offset="1" stop-color="var(--a2)" /></linearGradient>
  </defs>
  <path d="M16 3.5 27 9.8v12.4L16 28.500 5 22.200V9.800z" fill="none" stroke="url(#{id})" stroke-width="1.8" stroke-linejoin="round" />
  <path d="M16 22.500c0-6.500 3.200-10 7.500-11-1 5.500-3.300 9-7.500 11zM16 22.500c0-4.200-2.200-6.600-5.500-7.600.6 3.400 2.200 5.800 5.500 7.600z" fill="url(#{id})" />
</svg>
