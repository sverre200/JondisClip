<script lang="ts" module>
  export type View = "all" | "unsorted" | string;
</script>

<script lang="ts">
  import type { Folder } from "./api";
  import { drag, startDrag } from "./drag.svelte";

  // Folder navigation (ADR-0014). Rows are drop targets for clips
  // ([data-drop-folder]) and can be dragged to reorder.

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
    onreorder,
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
    onreorder: (ids: string[]) => void;
    ontoggle: () => void;
  } = $props();

  let list = $state<HTMLElement>();
  const rowEls: Record<string, HTMLElement | undefined> = $state({});

  const over = $derived(drag.target?.type === "folder" ? drag.target.id : undefined);
  const insertLine = $derived(drag.target?.type === "folder-insert" ? drag.target.rect : null);

  function folderDown(e: PointerEvent, f: Folder) {
    startDrag(
      e,
      () => ({ kind: "folder", id: f.id, label: f.name, color: f.color }),
      (t, p) => {
        if (t.type !== "folder-insert" || p.kind !== "folder") return;
        const ids = folders.map((x) => x.id).filter((id) => id !== p.id);
        const at = t.before ? ids.indexOf(t.before) : ids.length;
        ids.splice(at < 0 ? ids.length : at, 0, p.id);
        onreorder(ids);
      },
      { folders: { rows: () => folders.map((x) => ({ id: x.id, el: rowEls[x.id]! })).filter((r) => r.el), area: () => list } },
    );
  }

  function key(e: KeyboardEvent, i: number) {
    // Up/down between rows; Alt+Up/Down moves a folder.
    const rows = [...(list?.querySelectorAll<HTMLButtonElement>("button.row") ?? [])];
    const cur = rows.indexOf(e.currentTarget as HTMLButtonElement);
    if ((e.key === "ArrowDown" || e.key === "ArrowUp") && e.altKey && i >= 0) {
      e.preventDefault();
      const ids = folders.map((f) => f.id);
      const j = e.key === "ArrowDown" ? i + 1 : i - 1;
      if (j < 0 || j >= ids.length) return;
      [ids[i], ids[j]] = [ids[j]!, ids[i]!];
      onreorder(ids);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      rows[Math.min(rows.length - 1, cur + 1)]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      rows[Math.max(0, cur - 1)]?.focus();
    }
  }
</script>

<nav class="side" class:collapsed aria-label="Folders">
  <div class="head">
    {#if !collapsed}<span class="title">Folders</span>{/if}
    <button type="button" class="icon" onclick={ontoggle} title={collapsed ? "Show folders" : "Hide folders"} aria-label={collapsed ? "Show folders" : "Hide folders"} aria-expanded={!collapsed}>
      <svg viewBox="0 0 16 16" aria-hidden="true">
        {#if collapsed}<path d="m6 3.5 4.5 4.5L6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />{:else}<path d="M10 3.5 5.5 8l4.5 4.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />{/if}
      </svg>
    </button>
  </div>

  <div class="list" bind:this={list}>
    <button type="button" class="row" class:on={view === "all"} aria-current={view === "all" ? "page" : undefined} title="All clips" onclick={() => onview("all")} onkeydown={(e) => key(e, -1)}>
      <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true"><rect x="2" y="2.5" width="5" height="5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4" /><rect x="9" y="2.5" width="5" height="5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4" /><rect x="2" y="9.5" width="5" height="5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4" /><rect x="9" y="9.5" width="5" height="5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4" /></svg>
      {#if !collapsed}<span class="name">All clips</span><span class="n">{allCount}</span>{/if}
    </button>
    <button
      type="button"
      class="row"
      class:on={view === "unsorted"}
      class:drop={over === null}
      data-drop-folder=""
      aria-current={view === "unsorted" ? "page" : undefined}
      title="Not in a folder (drop clips here to take them out of their folder)"
      onclick={() => onview("unsorted")}
      onkeydown={(e) => key(e, -1)}
    >
      <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 4.5h11v8a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1z M2.5 4.5 4 2.5h8l1.5 2" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" /></svg>
      {#if !collapsed}<span class="name">Not in a folder</span><span class="n">{unsortedCount}</span>{/if}
    </button>

    {#if folders.length}<div class="sep"></div>{/if}

    {#each folders as f, i (f.id)}
      <button
        type="button"
        class="row folder"
        class:on={view === f.id}
        class:drop={over === f.id}
        class:lifted={drag.payload?.kind === "folder" && drag.payload.id === f.id}
        data-drop-folder={f.id}
        aria-current={view === f.id ? "page" : undefined}
        title={f.name}
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
          } else key(e, i);
        }}
      >
        <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true" style:color="var(--folder-{f.color})">
          <path d="M1.75 4.25a1 1 0 0 1 1-1h3.4l1.4 1.5h5.7a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1H2.75a1 1 0 0 1-1-1z" fill="currentColor" fill-opacity="0.22" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" />
        </svg>
        {#if !collapsed}<span class="name">{f.name}</span><span class="n">{counts[f.id] ?? 0}</span>{/if}
      </button>
    {/each}

    {#if insertLine}
      <div class="insert" style:top="{insertLine.top - (list?.getBoundingClientRect().top ?? 0) + (list?.scrollTop ?? 0)}px"></div>
    {/if}
  </div>

  <button type="button" class="row new" onclick={oncreate} title="New folder">
    <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3.5v9M3.5 8h9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
    {#if !collapsed}<span class="name">New folder</span>{/if}
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
