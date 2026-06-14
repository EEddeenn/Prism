/* Single source of truth for the side-rail tabs (id + label), so the shell and
 * the store agree and `activeTab` is a typed union, not a bare string. */

export const TABS = [
  { id: 'overview', label: 'Overview' },
  { id: 'structure', label: 'Structure' },
  { id: 'diff', label: 'Diff' },
  { id: 'risk', label: 'Risk' },
  { id: 'node', label: 'Node' },
  { id: 'duplicates', label: 'Dups' },
  { id: 'cycles', label: 'Cycles' },
  { id: 'tda', label: 'TDA' },
] as const;

export type TabId = (typeof TABS)[number]['id'];

export const TAB_LABELS: Record<TabId, string> = Object.fromEntries(
  TABS.map((t) => [t.id, t.label]),
) as Record<TabId, string>;
