// Song list selection (F9.2): click, ctrl-click and shift-click, like Windows Explorer.

/** New selection after clicking row `index` of `ids` with `mods` ({ ctrl, shift }). */
export function select(state, ids, index, mods = {}) {
  const prev = state?.selected ?? new Set();
  const anchor = state?.anchor ?? index;
  if (mods.shift) {
    const [from, to] = anchor <= index ? [anchor, index] : [index, anchor];
    const range = ids.slice(from, to + 1);
    const selected = new Set(mods.ctrl ? [...prev, ...range] : range);
    return { selected, anchor };
  }
  if (mods.ctrl) {
    const selected = new Set(prev);
    if (selected.has(ids[index])) selected.delete(ids[index]);
    else selected.add(ids[index]);
    return { selected, anchor: index };
  }
  return { selected: new Set([ids[index]]), anchor: index };
}
