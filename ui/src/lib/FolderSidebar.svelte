<script lang="ts" module>
  export type View = "all" | "unsorted" | string;
</script>

<script lang="ts">
  import { t } from "./i18n.svelte";
  import { MAX_DEPTH, isWithin, type Folder } from "./api";
  import { drag, startDrag } from "./drag.svelte";

  // Folder navigation (ADR-0014/0015) as a tree. Rows are drop targets for
  // clips ([data-drop-folder]); a folder can be dragged between rows (order,
  // or out of its parent) or onto another folder (inside it).

  let {
    folders,
    counts,
    allCount,
    unsortedCount,
    view,
    collapsed,
    onview,
    oncreate,
    oncontext,
    onplace,
    ontoggle,
  }: {
    folders: Folder[];
    counts: Record<string, number>;
    allCount: number;
    unsortedCount: number;
    view: View;
    collapsed: boolean;
    onview: (v: View) => void;
    oncreate: () => void;
    oncontext: (e: MouseEvent, f: Folder) => void;
    /** put folder `id` inside `parent` (null: top level), before sibling `before` (null: last) */
    onplace: (id: string, parent: string | null, before: string | null) => void;
    ontoggle: () => void;
  } = $props();

  let list = $state<HTMLElement>();
  const rowEls: Record<string, HTMLElement | undefined> = $state({});

  // Collapsed folders (a UI preference). A folder with subfolders starts open.
  let closed = $state<string[]>(JSON.parse(localStorage.getItem("jc.library.closed") ?? "[]") as string[]);
  $effect(() => localStorage.setItem("jc.library.closed", JSON.stringify(closed)));
  const hasKids = $derived(new Set(folders.map((f) => f.parent).filter((p): p is string => !!p)));
  function toggle(id: string) {
    closed = closed.includes(id) ? closed.filter((x) => x !== id) : [...closed, id];
  }
  // The open folder's parents are always expanded.
  $effect(() => {
    const v = view;
    if (closed.some((c) => isWithin(v, c))) closed = closed.filter((c) => !isWithin(v, c));
  });

  /** Tree order (parents before children, siblings in sidebar order), hidden
   *  rows of collapsed folders left out. */
  const rows = $derived.by(() => {
    const out: Folder[] = [];
    const add = (parent: string | null) => {
      for (const f of folders) {
        if (f.parent !== parent) continue;
        out.push(f);
        if (!closed.includes(f.id)) add(f.id);
      }
    };
    add(null);
    // Orphans (parent missing on disk) at the end, so nothing is lost.
    for (const f of folders) if (!out.includes(f) && !folders.some((x) => x.id === f.parent)) out.push(f);
    return out;
  });

  const over = $derived(drag.target?.type === "folder" ? drag.target.id : undefined);
  const insertLine = $derived(drag.target?.type === "folder-insert" ? drag.target.rect : null);

  /** How many levels a folder's subtree spans (1 = no subfolders). */
  const span = (id: string) => Math.max(1, ...folders.filter((x) => isWithin(x.id, id)).map((x) => x.depth - (folders.find((f) => f.id === id)?.depth ?? 1) + 1));

  function folderDown(e: PointerEvent, f: Folder) {
    startDrag(
      e,
      () => ({ kind: "folder", id: f.id, label: f.leaf, color: f.color }),
      (t, p) => {
        if (p.kind !== "folder") return;
        if (t.type === "folder" && t.id) {
          onplace(p.id, t.id, null);
        } else if (t.type === "folder-insert") {
          // Between rows: next to the row below, at its level (end: top level).
          const below = t.before ? folders.find((x) => x.id === t.before) : null;
          if (below && (below.id === p.id || isWithin(below.id, p.id))) return;
          onplace(p.id, below?.parent ?? null, below?.id ?? null);
        }
      },
      {
        folders: {
          rows: () => rows.map((x) => ({ id: x.id, el: rowEls[x.id]! })).filter((r) => r.el),
          area: () => list,
          canNest: (id) => {
            const target = folders.find((x) => x.id === id);
            return !!target && id !== f.id && !isWithin(id, f.id) && target.depth + span(f.id) <= MAX_DEPTH;
          },
        },
      },
    );
  }

  function key(e: KeyboardEvent, f: Folder | null) {
    // Up/down between rows; Alt+Up/Down moves a folder among its siblings;
    // Right/Left open and close a folder with subfolders.
    const els = [...(list?.querySelectorAll<HTMLButtonElement>("button.row") ?? [])];
    const cur = els.indexOf(e.currentTarget as HTMLButtonElement);
    if ((e.key === "ArrowDown" || e.key === "ArrowUp") && e.altKey && f) {
      e.preventDefault();
      const sibs = folders.filter((x) => x.parent === f.parent);
      const i = sibs.indexOf(f);
      if (e.key === "ArrowUp" && i > 0) onplace(f.id, f.parent, sibs[i - 1]!.id);
      else if (e.key === "ArrowDown" && i < sibs.length - 1) onplace(f.id, f.parent, sibs[i + 2]?.id ?? null);
    } else if (f && hasKids.has(f.id) && (e.key === "ArrowRight" || e.key === "ArrowLeft")) {
      e.preventDefault();
      if ((e.key === "ArrowLeft") !== closed.includes(f.id)) toggle(f.id);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      els[Math.min(els.length - 1, cur + 1)]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      els[Math.max(0, cur - 1)]?.focus();
    }
  }
</script>

<nav class="side" class:collapsed aria-label={t("Folders")}>
  <div class="head">
    {#if !collapsed}<span class="title">{t("Folders")}</span>{/if}
    <button type="button" class="icon" onclick={ontoggle} title={collapsed ? t("Show folders") : t("Hide folders")} aria-label={collapsed ? t("Show folders") : t("Hide folders")} aria-expanded={!collapsed}>
      <svg viewBox="0 0 16 16" aria-hidden="true">
        {#if collapsed}<path d="m6 3.5 4.5 4.5L6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />{:else}<path d="M10 3.5 5.5 8l4.5 4.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />{/if}
      </svg>
    </button>
  </div>

  <div class="list" bind:this={list}>
    <button type="button" class="row" class:on={view === "all"} aria-current={view === "all" ? "page" : undefined} title={t("All clips")} onclick={() => onview("all")} onkeydown={(e) => key(e, null)}>
      <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true"><rect x="2" y="2.5" width="5" height="5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4" /><rect x="9" y="2.5" width="5" height="5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4" /><rect x="2" y="9.5" width="5" height="5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4" /><rect x="9" y="9.5" width="5" height="5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4" /></svg>
      {#if !collapsed}<span class="name">{t("All clips")}</span><span class="n">{allCount}</span>{/if}
    </button>
    <button
      type="button"
      class="row"
      class:on={view === "unsorted"}
      class:drop={over === null}
      data-drop-folder=""
      aria-current={view === "unsorted" ? "page" : undefined}
      title={t("Not in a folder (drop clips here to take them out of their folder)")}
      onclick={() => onview("unsorted")}
      onkeydown={(e) => key(e, null)}
    >
      <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 4.5h11v8a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1z M2.5 4.5 4 2.5h8l1.5 2" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" /></svg>
      {#if !collapsed}<span class="name">{t("Not in a folder")}</span><span class="n">{unsortedCount}</span>{/if}
    </button>

    {#if folders.length}<div class="sep"></div>{/if}

    {#each rows as f (f.id)}
      <button
        type="button"
        class="row folder"
        class:on={view === f.id}
        class:drop={over === f.id}
        class:lifted={drag.payload?.kind === "folder" && drag.payload.id === f.id}
        data-drop-folder={f.id}
        aria-current={view === f.id ? "page" : undefined}
        aria-expanded={hasKids.has(f.id) ? !closed.includes(f.id) : undefined}
        title={f.name.replaceAll("\\", " › ")}
        style:--indent="{collapsed ? 0 : (f.depth - 1) * 14}px"
        bind:this={rowEls[f.id]}
        onclick={() => onview(f.id)}
        oncontextmenu={(e) => {
          e.preventDefault();
          oncontext(e, f);
        }}
        onpointerdown={(e) => folderDown(e, f)}
        onkeydown={(e) => {
          if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
            e.preventDefault();
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            oncontext(new MouseEvent("contextmenu", { clientX: r.left + 24, clientY: r.bottom }), f);
          } else key(e, f);
        }}
      >
        {#if !collapsed}
          {#if hasKids.has(f.id)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <span
              class="twist"
              class:open={!closed.includes(f.id)}
              aria-hidden="true"
              onpointerdown={(e) => e.stopPropagation()}
              onclick={(e) => {
                e.stopPropagation();
                toggle(f.id);
              }}
            >
              <svg viewBox="0 0 16 16"><path d="m6 4 4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
            </span>
          {:else}<span class="twist none" aria-hidden="true"></span>{/if}
        {/if}
        <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true" style:color="var(--folder-{f.color})">
          <path d="M1.75 4.25a1 1 0 0 1 1-1h3.4l1.4 1.5h5.7a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1H2.75a1 1 0 0 1-1-1z" fill="currentColor" fill-opacity="0.22" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" />
        </svg>
        {#if !collapsed}<span class="name">{f.leaf}</span><span class="n">{counts[f.id] ?? 0}</span>{/if}
      </button>
    {/each}

    {#if insertLine}
      <div class="insert" style:top="{insertLine.top - (list?.getBoundingClientRect().top ?? 0) + (list?.scrollTop ?? 0)}px"></div>
    {/if}
  </div>

  <button type="button" class="row new" onclick={oncreate} title={t("New folder")}>
    <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3.5v9M3.5 8h9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
    {#if !collapsed}<span class="name">{t("New folder")}</span>{/if}
  </button>
</nav>

<style>
  .side {
    display: flex;
    flex-direction: column;
    width: 208px;
    flex: none;
    border-right: 1px solid var(--border);
    padding: var(--s2) var(--s2) var(--s3);
    min-height: 0;
  }
  .side.collapsed {
    width: 52px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 30px;
    padding: 0 var(--s1) 0 var(--s2);
    margin-bottom: var(--s1);
  }
  .collapsed .head {
    justify-content: center;
    padding: 0;
  }
  .title {
    font-size: var(--fs-xs);
    font-weight: var(--fw-strong);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .icon {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-faint);
    cursor: pointer;
  }
  .icon:hover {
    background: var(--surface-hover);
    color: var(--text);
  }
  .icon svg {
    width: 14px;
    height: 14px;
  }
  .list {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s2);
    width: 100%;
    height: 32px;
    padding: 0 var(--s2);
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
    flex: none;
  }
  .collapsed .row {
    justify-content: center;
    padding: 0;
  }
  .row:hover {
    background: var(--surface-hover);
    color: var(--text);
  }
  .row.on {
    background: var(--surface-raised);
    color: var(--text);
    font-weight: var(--fw-strong);
  }
  .row.drop {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--text);
  }
  .row.lifted {
    opacity: 0.4;
  }
  .row.folder {
    padding-left: calc(var(--s1) + var(--indent, 0px));
    gap: var(--s1);
  }
  .row.folder .glyph {
    margin-right: var(--s1);
  }
  .collapsed .row.folder {
    padding-left: 0;
  }
  .twist {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    flex: none;
    border-radius: var(--radius-sm);
    color: var(--text-faint);
  }
  .twist:not(.none):hover {
    background: var(--surface-press);
    color: var(--text);
  }
  .twist svg {
    width: 12px;
    height: 12px;
    transition: transform var(--t-fast) var(--ease);
  }
  .twist.open svg {
    transform: rotate(90deg);
  }
  .glyph {
    width: 16px;
    height: 16px;
    flex: none;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .n {
    color: var(--text-faint);
    font-size: var(--fs-xs);
    font-weight: var(--fw-regular);
    font-variant-numeric: tabular-nums;
  }
  .sep {
    height: 1px;
    margin: var(--s2) var(--s2);
    background: var(--border);
    flex: none;
  }
  .new {
    margin-top: var(--s2);
    color: var(--text-faint);
  }
  .insert {
    position: absolute;
    left: var(--s2);
    right: var(--s2);
    height: 2px;
    margin-top: -1px;
    border-radius: 1px;
    background: var(--accent);
    pointer-events: none;
  }
</style>
