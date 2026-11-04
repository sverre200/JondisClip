// Pointer-based drag and drop for the library (ADR-0014): no HTML5 DnD (it
// fights WebView2's own file drops) and no library. A 6 px threshold tells a
// click from a drag; while dragging, one hit test per pointer move decides
// the target. Nothing runs when no drag is in progress.

export type Payload =
  | { kind: "clips"; ids: string[]; keys: string[]; label: string; thumb: string | null }
  | { kind: "folder"; id: string; label: string; color: string };

/** Where a drop would land. */
export type Target =
  | { type: "folder"; id: string | null } // null = "Not in a folder"
  | { type: "insert"; before: string | null; rect: { left: number; top: number; height: number } }
  | { type: "folder-insert"; before: string | null; rect: { left: number; top: number; width: number } };

export const drag = $state<{
  payload: Payload | null;
  x: number;
  y: number;
  target: Target | null;
  /** shown under the ghost when a drop isn't possible here */
  hint: string | null;
}>({ payload: null, x: 0, y: 0, target: null, hint: null });

export interface Reorder {
  /** cards in display order, with their clip key */
  cards: () => { key: string; el: HTMLElement }[];
  /** the scrollable grid container */
  area: () => HTMLElement | null | undefined;
  /** why reordering isn't possible right now (search/filter active…) */
  blocked: () => string | null;
}

export interface FolderReorder {
  rows: () => { id: string; el: HTMLElement }[];
  area: () => HTMLElement | null | undefined;
}

const THRESHOLD = 6;

/** Call from pointerdown. Starts a drag once the pointer moves 6 px. */
export function startDrag(
  e: PointerEvent,
  payload: () => Payload | null,
  ondrop: (t: Target, p: Payload) => void,
  opts: { reorder?: Reorder; folders?: FolderReorder } = {},
) {
  if (e.button !== 0 || e.ctrlKey || e.shiftKey || e.metaKey) return;
  const sx = e.clientX;
  const sy = e.clientY;
  let active = false;
  let raf = 0;
  let edge = 0; // autoscroll speed
  let lastX = sx;
  let lastY = sy;
  let hitFrame = 0;

  const scrollLoop = () => {
    const area = opts.reorder?.area() ?? opts.folders?.area();
    if (area && edge !== 0) {
      area.scrollTop += edge;
      // The cards moved under a still pointer: aim again.
      hitTest(lastX, lastY, opts);
    }
    raf = edge !== 0 ? requestAnimationFrame(scrollLoop) : 0;
  };

  const move = (ev: PointerEvent) => {
    if (!active) {
      if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < THRESHOLD) return;
      const p = payload();
      if (!p) return cleanup();
      active = true;
      drag.payload = p;
      document.body.classList.add("dragging");
    }
    drag.x = lastX = ev.clientX;
    drag.y = lastY = ev.clientY;
    // At most one hit test per frame (pointer events can come at 240 Hz).
    if (!hitFrame) {
      hitFrame = requestAnimationFrame(() => {
        hitFrame = 0;
        if (drag.payload) hitTest(lastX, lastY, opts);
      });
    }
    // Auto-scroll near the top/bottom of the grid.
    const area = opts.reorder?.area();
    if (area) {
      const r = area.getBoundingClientRect();
      edge = ev.clientY < r.top + 48 ? -12 : ev.clientY > r.bottom - 48 ? 12 : 0;
      if (edge !== 0 && !raf) raf = requestAnimationFrame(scrollLoop);
    }
  };

  const up = () => {
    // Aim with the final position (a pending frame may not have run).
    if (active && drag.payload) hitTest(lastX, lastY, opts);
    const p = drag.payload;
    const t = drag.target;
    const was = active;
    cleanup();
    if (was && p && t) ondrop(t, p);
  };

  const key = (ev: KeyboardEvent) => {
    if (ev.key === "Escape" && active) {
      ev.preventDefault();
      ev.stopPropagation();
      cleanup();
    }
  };

  // A drag must not also count as a click on the card it started on.
  const swallowClick = (ev: MouseEvent) => {
    ev.stopPropagation();
    ev.preventDefault();
  };

  // Losing focus mid-drag (Alt+Tab, button released outside the window)
  // cancels it, so a later click can never complete a stale drop.
  const lost = () => cleanup();

  function cleanup() {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cleanup);
    window.removeEventListener("keydown", key, true);
    window.removeEventListener("blur", lost);
    document.removeEventListener("visibilitychange", lost);
    cancelAnimationFrame(raf);
    cancelAnimationFrame(hitFrame);
    hitFrame = 0;
    raf = 0;
    edge = 0;
    if (active) {
      window.addEventListener("click", swallowClick, { capture: true, once: true });
      setTimeout(() => window.removeEventListener("click", swallowClick, { capture: true }), 0);
    }
    active = false;
    drag.payload = null;
    drag.target = null;
    drag.hint = null;
    document.body.classList.remove("dragging");
  }

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cleanup);
  window.addEventListener("keydown", key, true);
  window.addEventListener("blur", lost);
  document.addEventListener("visibilitychange", lost);
}

function hitTest(x: number, y: number, opts: { reorder?: Reorder; folders?: FolderReorder }) {
  const p = drag.payload;
  if (!p) return;
  drag.hint = null;
  const under = document.elementFromPoint(x, y);

  if (p.kind === "folder") {
    // Folders only reorder among themselves.
    const rows = opts.folders?.rows() ?? [];
    const area = opts.folders?.area();
    if (!area || !under || !area.contains(under) || rows.length === 0) {
      drag.target = null;
      return;
    }
    let before: string | null = null;
    let line = rows[rows.length - 1]!.el.getBoundingClientRect().bottom;
    for (const r of rows) {
      const b = r.el.getBoundingClientRect();
      if (y < b.top + b.height / 2) {
        before = r.id;
        line = b.top;
        break;
      }
    }
    const a = rows[0]!.el.getBoundingClientRect();
    drag.target = before === p.id ? null : { type: "folder-insert", before, rect: { left: a.left, top: line - 1, width: a.width } };
    return;
  }

  // Clips onto a folder in the sidebar.
  const drop = under?.closest<HTMLElement>("[data-drop-folder]");
  if (drop) {
    const id = drop.dataset.dropFolder ?? "";
    drag.target = { type: "folder", id: id === "" ? null : id };
    return;
  }

  // Clips within the grid: reorder, when allowed.
  const ro = opts.reorder;
  const area = ro?.area();
  if (ro && area && under && area.contains(under)) {
    const why = ro.blocked();
    if (why) {
      drag.target = null;
      drag.hint = why;
      return;
    }
    const cards = ro.cards();
    if (cards.length === 0) {
      drag.target = null;
      return;
    }
    // Usually the pointer is on a card: take it directly. Only in the gaps
    // between cards search for the nearest (bounded by what's on screen).
    const onCard = under.closest<HTMLElement>("[data-key]");
    const direct = onCard ? cards.find((c) => c.el === onCard) : undefined;
    let best = direct ?? cards[0]!;
    let bestD = direct ? 0 : Infinity;
    const view = area.getBoundingClientRect();
    for (const c of direct ? [] : cards) {
      const r = c.el.getBoundingClientRect();
      if (r.bottom < view.top || r.top > view.bottom) continue; // off screen
      const cx = Math.max(r.left, Math.min(x, r.right));
      const cy = Math.max(r.top, Math.min(y, r.bottom));
      const d = Math.hypot(x - cx, y - cy);
      if (d < bestD) {
        bestD = d;
        best = c;
      }
    }
    const r = best.el.getBoundingClientRect();
    const after = x > r.left + r.width / 2;
    const i = cards.indexOf(best);
    const before = after ? (cards[i + 1]?.key ?? null) : best.key;
    // Dropping a clip right where it already is: no target.
    if (p.keys.includes(best.key) && (p.keys.length === 1 ? before === p.keys[0] || after : true)) {
      drag.target = null;
      return;
    }
    const left = after ? r.right + 6 : r.left - 10;
    drag.target = { type: "insert", before, rect: { left, top: r.top, height: r.height } };
    return;
  }
  drag.target = null;
}
