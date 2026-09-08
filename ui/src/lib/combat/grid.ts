// why: the spreadsheet half of a grid, shared by every combat table --
// sort by any column, remember which columns are shown. Pure, no DOM.

export type Dir = 1 | -1;

/** why: numbers sort with nulls last whichever way; strings case-insensitive */
export function sortRows<T>(rows: T[], key: keyof T, dir: Dir): T[] {
  return [...rows].sort((a, b) => {
    const x = a[key] as unknown;
    const y = b[key] as unknown;
    if (x == null && y == null) return 0;
    if (x == null) return 1;
    if (y == null) return -1;
    if (typeof x === 'number' && typeof y === 'number') return (x - y) * dir;
    return String(x).localeCompare(String(y), undefined, { sensitivity: 'base' }) * dir;
  });
}

/** why: the next sort state on a header click -- a new column starts
 * descending for numbers (biggest first is what a meter wants) */
export function nextSort<K extends string>(cur: { key: K; dir: Dir }, key: K, numeric: boolean): { key: K; dir: Dir } {
  if (cur.key === key) return { key, dir: cur.dir === 1 ? -1 : 1 };
  return { key, dir: numeric ? -1 : 1 };
}

// ponytail: localStorage, per machine -- move into preferences.json if
// it ever needs to follow the profile
export function loadCols(store: string, defaults: string[]): Set<string> {
  try {
    const raw = localStorage.getItem(`eqlp.cols.${store}`);
    if (raw) return new Set(JSON.parse(raw) as string[]);
  } catch {
    // why: private mode / blocked storage -- defaults are fine
  }
  return new Set(defaults);
}

export function saveCols(store: string, cols: Set<string>) {
  try {
    localStorage.setItem(`eqlp.cols.${store}`, JSON.stringify([...cols]));
  } catch {
    // why: nothing to do -- the choice just doesn't survive a restart
  }
}
