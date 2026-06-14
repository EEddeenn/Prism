'use client';

/* Cmd/Ctrl-K command palette. A self-contained overlay: it owns its open state,
 * installs a single global keydown listener, and runs against the shared store.
 * Mounted once by the page. ArrowUp/Down move the active row across the flat
 * filtered list (groups are visual only); Enter runs it; Escape closes. */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { shortLabel } from '@/lib/palette';

type Group = 'Packages' | 'Runs' | 'Cycles' | 'Duplicate families' | 'Actions';

interface Item {
  group: Group;
  label: string;
  hint?: string;
  /** A small right-aligned badge (e.g. package depth). */
  badge?: string;
  /** Show a pink dot before the badge (duplicate-version packages). */
  dot?: boolean;
  run: () => void;
}

const CAP = 8;

export default function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [term, setTerm] = useState('');
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const prevFocus = useRef<HTMLElement | null>(null);

  // Pull just what the items need; the store keeps 2D + 3D in lockstep.
  const nodes = useGraphStore((s) => s.nodes);
  const runs = useGraphStore((s) => s.runs);
  const sccs = useGraphStore((s) => s.sccs);
  const duplicates = useGraphStore((s) => s.duplicates);
  const dupVersionIds = useGraphStore((s) => s.dupVersionIds);
  const selectNode = useGraphStore((s) => s.selectNode);
  const requestFocus = useGraphStore((s) => s.requestFocus);
  const selectRun = useGraphStore((s) => s.selectRun);
  const highlightSet = useGraphStore((s) => s.highlightSet);
  const setView = useGraphStore((s) => s.setView);
  const setMode3d = useGraphStore((s) => s.setMode3d);
  const setEdgeFilter = useGraphStore((s) => s.setEdgeFilter);
  const clearSelection = useGraphStore((s) => s.clearSelection);

  // Map a duplicate cluster's versions (by purl) onto live occurrence ids.
  const purlToId = useMemo(() => {
    const m = new Map<string, number>();
    for (const n of nodes) m.set(n.purl, n.id);
    return m;
  }, [nodes]);

  // Global hotkeys: Cmd/Ctrl+K (or "/" when not typing) opens; Escape closes.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const k = e.key;
      if ((e.metaKey || e.ctrlKey) && (k === 'k' || k === 'K')) {
        e.preventDefault();
        setOpen((v) => !v);
        return;
      }
      if (k === 'Escape') {
        setOpen(false);
        return;
      }
      if (k === '/' && !e.metaKey && !e.ctrlKey && !e.altKey) {
        const el = document.activeElement as HTMLElement | null;
        const tag = el?.tagName;
        const typing =
          tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el?.isContentEditable;
        if (!typing) {
          e.preventDefault();
          setOpen(true);
        }
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // On open: reset, lock background scroll, focus the input. On close: restore
  // background scroll and return focus to wherever it was.
  useEffect(() => {
    if (!open) return;
    prevFocus.current = document.activeElement as HTMLElement | null;
    setTerm('');
    setActive(0);
    document.body.style.overflow = 'hidden';
    const raf = requestAnimationFrame(() => inputRef.current?.focus());
    return () => {
      cancelAnimationFrame(raf);
      document.body.style.overflow = '';
      prevFocus.current?.focus?.();
    };
  }, [open]);

  const items = useMemo<Item[]>(() => {
    const q = term.trim().toLowerCase();
    const match = (s: string) => !q || s.toLowerCase().includes(q);
    const out: Item[] = [];

    // Packages — fuzzy substring over the full purl.
    let pkgN = 0;
    for (const n of nodes) {
      if (pkgN >= CAP) break;
      if (!match(n.purl)) continue;
      pkgN++;
      out.push({
        group: 'Packages',
        label: shortLabel(n.purl),
        hint: n.purl,
        badge: `d${n.depth}`,
        dot: dupVersionIds.has(n.version_id),
        run: () => {
          // The treemap doesn't pan/recenter, so jump to a graph view first.
          if (useGraphStore.getState().view === 'map') setView('2d');
          void selectNode(n.id);
          requestFocus(n.id);
        },
      });
    }

    // Runs.
    let runN = 0;
    for (const r of runs) {
      if (runN >= CAP) break;
      const lbl = `run ${r.run}`;
      const meta = r.nodes != null ? `${r.nodes}n/${r.edges}e` : '';
      if (!match(lbl) && !match(meta)) continue;
      runN++;
      out.push({
        group: 'Runs',
        label: lbl,
        hint: meta || undefined,
        run: () => void selectRun(r.run),
      });
    }

    // Cycles — each true cycle SCC; highlight its members.
    let cycN = 0;
    const cycles = sccs.filter((c) => c.is_cycle);
    cycles.forEach((c, i) => {
      if (cycN >= CAP) return;
      const names = c.members.map((m) => shortLabel(m.purl)).join(' → ');
      const lbl = `Cycle ${i + 1}`;
      if (!match(lbl) && !match(names)) return;
      cycN++;
      const ids = c.members.map((m) => m.id);
      out.push({
        group: 'Cycles',
        label: lbl,
        hint: names,
        badge: `${c.members.length}`,
        run: () => highlightSet(ids, 'cycle'),
      });
    });

    // Duplicate families — map versions (by purl) onto occurrence ids.
    let dupN = 0;
    for (const c of duplicates) {
      if (dupN >= CAP) break;
      if (!match(c.package_purl)) continue;
      const ids = c.versions
        .map((v) => purlToId.get(v.purl))
        .filter((x): x is number => x != null);
      if (!ids.length) continue;
      dupN++;
      out.push({
        group: 'Duplicate families',
        label: shortLabel(c.package_purl),
        hint: c.package_purl,
        badge: `${c.version_count}v`,
        run: () => highlightSet(ids, c.package_purl),
      });
    }

    // Actions — static commands. Filtered like everything else.
    const actions: Item[] = [
      { group: 'Actions', label: 'Switch to 2D', run: () => setView('2d') },
      { group: 'Actions', label: 'Switch to 3D', run: () => setView('3d') },
      { group: 'Actions', label: 'Treemap view', run: () => setView('map') },
      { group: 'Actions', label: '3D: Layered', run: () => { setMode3d('layered'); setView('3d'); } },
      { group: 'Actions', label: '3D: Fiber', run: () => { setMode3d('fiber'); setView('3d'); } },
      { group: 'Actions', label: '3D: Terrain', run: () => { setMode3d('terrain'); setView('3d'); } },
      { group: 'Actions', label: '3D: TDA', run: () => { setMode3d('tda'); setView('3d'); } },
      { group: 'Actions', label: 'Reset selection', run: () => clearSelection() },
      { group: 'Actions', label: 'Edges: runtime only', run: () => setEdgeFilter('runtime') },
      { group: 'Actions', label: 'Edges: all', run: () => setEdgeFilter('all') },
    ];
    let actN = 0;
    for (const a of actions) {
      if (actN >= CAP) break;
      if (!match(a.label)) continue;
      actN++;
      out.push(a);
    }

    return out;
  }, [
    term, nodes, runs, sccs, duplicates, dupVersionIds, purlToId,
    selectNode, requestFocus, selectRun, highlightSet, setView, setMode3d,
    setEdgeFilter, clearSelection,
  ]);

  // Keep the active row in range whenever the result list changes.
  useEffect(() => {
    setActive((a) => (items.length ? Math.min(a, items.length - 1) : 0));
  }, [items.length]);

  // Scroll the active row into view as it moves.
  useEffect(() => {
    if (!open) return;
    const el = listRef.current?.querySelector<HTMLElement>(`[data-idx="${active}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  }, [active, open]);

  if (!open) return null;

  function close() {
    setOpen(false);
  }

  function runItem(i: number) {
    const it = items[i];
    if (!it) return;
    it.run();
    close();
  }

  function onInputKey(e: React.KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((a) => (items.length ? (a + 1) % items.length : 0));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((a) => (items.length ? (a - 1 + items.length) % items.length : 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      runItem(active);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation(); // don't also trigger the window-level Escape handler
      close();
    }
  }

  // Render the flat list but draw a group heading whenever the group changes.
  let prevGroup: Group | null = null;

  return (
    <div
      className="fixed inset-0 z-50 flex justify-center bg-black/55"
      role="presentation"
      onMouseDown={close}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        className="mt-[12vh] w-[min(640px,92vw)] max-h-[70vh] flex flex-col rounded-xl border border-line-2 bg-panel shadow-panel overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <input
          ref={inputRef}
          type="search"
          autoComplete="off"
          spellCheck={false}
          placeholder="Search packages, runs, cycles, actions…"
          aria-label="Command palette search"
          role="combobox"
          aria-expanded={items.length > 0}
          aria-controls="command-palette-list"
          aria-autocomplete="list"
          aria-activedescendant={items.length ? `cp-opt-${active}` : undefined}
          value={term}
          onChange={(e) => {
            setTerm(e.target.value);
            setActive(0);
          }}
          onKeyDown={onInputKey}
          className="w-full px-4 py-3 text-sm bg-panel-2 border-0 border-b border-line rounded-none placeholder:text-muted"
        />

        <div
          ref={listRef}
          id="command-palette-list"
          role="listbox"
          aria-label="Search results"
          className="overflow-y-auto min-h-0 py-1"
        >
          {!items.length ? (
            <div className="px-4 py-6 text-center text-sm text-muted">No matches.</div>
          ) : (
            items.map((it, i) => {
              const heading = it.group !== prevGroup ? it.group : null;
              prevGroup = it.group;
              const isActive = i === active;
              return (
                <div key={`${it.group}-${i}`}>
                  {heading && (
                    <div className="px-4 pt-2 pb-1 text-[10px] uppercase tracking-[1px] text-muted">
                      {heading}
                    </div>
                  )}
                  <div
                    data-idx={i}
                    id={`cp-opt-${i}`}
                    role="option"
                    aria-selected={isActive}
                    onMouseMove={() => active !== i && setActive(i)}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      runItem(i);
                    }}
                    className={`flex items-center gap-2 mx-1 px-3 py-1.5 rounded-md cursor-pointer ${
                      isActive ? 'bg-panel-3 text-ink' : 'text-ink/90 hover:bg-panel-2'
                    }`}
                  >
                    <span className="font-mono text-[12.5px] truncate">{it.label}</span>
                    {it.hint && it.hint !== it.label && (
                      <span className="text-[11px] text-muted truncate min-w-0 flex-1">
                        {it.hint}
                      </span>
                    )}
                    <span className="ml-auto flex items-center gap-1.5 shrink-0">
                      {it.dot && (
                        <span
                          className="inline-block w-1.5 h-1.5 rounded-full bg-node-dup"
                          title="duplicate version"
                        />
                      )}
                      {it.badge && (
                        <span className="text-[10px] text-muted bg-panel-2 border border-line rounded px-1.5 py-px">
                          {it.badge}
                        </span>
                      )}
                    </span>
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}
