<script lang="ts">
  import MetricsPanel from './MetricsPanel.svelte';
  import { apiOps } from '../lib/api-ops';
  import { bytes } from '../lib/format';
  import { host } from '../lib/stores.svelte';
  import type { ChartDef } from '../lib/metrics';

  const defs = $derived<ChartDef[]>([
    { id: 'cpu', label: 'Host CPU', icon: 'cpu', avg: 'cpu_avg', peak: 'cpu_max', max: 100, format: (v) => v.toFixed(0) + '%' },
    { id: 'mem', label: 'Host memory used', icon: 'ram', avg: 'mem_avg', peak: 'mem_max', max: host.status?.host.mem_total, color: 'var(--info)', format: (v) => bytes(v) },
    { id: 'disk', label: 'Disk free', icon: 'disk', avg: 'disk_free', peak: 'disk_free', color: 'var(--warn)', format: (v) => bytes(v), show: (m) => m.some((x) => x.disk_free != null) },
  ]);
</script>

<MetricsPanel title="Host history" icon="cpu" load={(r) => apiOps.hostMetrics(r)} {defs} />
