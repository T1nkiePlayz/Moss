// Kept apart from icons.ts so the mapping loads with the wizard, not with every page.
import type { IconName } from './icons';

/** Icon for a runtime in the wizard and lists: a per-game icon for games, else by category. */
const GAME_ICONS: Record<string, IconName> = {
  bedrock: 'layers', terraria: 'tree', factorio: 'settings', valheim: 'axe', gmod: 'wrench', cs2: 'crosshair', tf2: 'crosshair', l4d2: 'skull',
  rust: 'flame', palworld: 'paw', zomboid: 'skull', satisfactory: 'package',
  l4d: 'skull', css: 'crosshair', dods: 'crosshair', hl2dm: 'crosshair', nmrih: 'skull', ark: 'paw', sdtd: 'skull',
};
export function runtimeIcon(r: { id: string; category: string }): IconName {
  if (r.category === 'game') return GAME_ICONS[r.id] ?? 'gamepad';
  return r.category === 'proxy' ? 'link' : r.category === 'modded' ? 'puzzle' : r.category === 'custom' ? 'terminal' : 'cube';
}
