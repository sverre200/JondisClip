<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { api, FOLDER_COLORS, thumbUrl, type Clip, type Folder, type FolderColor, type LibraryView, type Organize } from "./lib/api";
  import { bytes, clock, dayLabel, fromWithin, whenShort } from "./lib/format";
  import Banner from "./lib/Banner.svelte";
  import Button from "./lib/Button.svelte";
  import ClipCard from "./lib/ClipCard.svelte";
  import ContextMenu, { type MenuEntry, type MenuItem } from "./lib/ContextMenu.svelte";
  import Dialog from "./lib/Dialog.svelte";
  import DragGhost from "./lib/DragGhost.svelte";
  import FolderSidebar, { type View } from "./lib/FolderSidebar.svelte";
  import Select from "./lib/Select.svelte";
  import { drag, startDrag, type Target, type Payload } from "./lib/drag.svelte";

  let {
    hotkey,
    refreshToken = 0,
    onscanned,
    onopen,
    onerror,
  }: {
    hotkey: string | null;
    /** bump to rescan (after an export) */
    refreshToken?: number;
    onscanned?: (clips: Clip[]) => void;
    onopen: (clip: Clip, all: Clip[]) => void;
    onerror: (msg: string) => void;
  } = $props();

  type Sort = "custom" | "newest" | "oldest" | "longest" | "largest";
  let view = $state<LibraryView | null>(null);
  let org = $state<Organize>({ version: 1, folders: [], titles: {} });
  let loadError = $state<string | null>(null);
  let refreshing = $state(false);
  let game = $state(""); // "" = all games
  let query = $state("");
  let sort = $state<Sort>("newest");
  // Selection: a set of clip ids plus the last-clicked one (anchor/primary).
  let selection = $state<string[]>([]);
  let selectedId = $state<string | null>(null);
  let renamingId = $state<string | null>(null);
  const cardEls: Record<string, HTMLDivElement | undefined> = $state({});
  let gridEl = $state<HTMLDivElement>();
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  type DialogState =
    | { kind: "new-folder"; moveKeys: string[]; error: string | null }
    | { kind: "rename-folder"; folder: Folder; error: string | null }
    | { kind: "delete-folder"; folder: Folder; count: number }
    | { kind: "delete-clips"; ids: string[]; label: string };
  let dialog = $state<DialogState | null>(null);
  let notice = $state<string | null>(null);
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  // The folder being viewed, and whether the sidebar is collapsed: UI
  // preferences, remembered across launches (the organisation itself lives
  // in organize.json).
  let folderView = $state<View>(localStorage.getItem("jc.library.view") ?? "all");
  let collapsed = $state(localStorage.getItem("jc.library.sidebar") === "collapsed" || (localStorage.getItem("jc.library.sidebar") === null && window.innerWidth < 900));
  $effect(() => localStorage.setItem("jc.library.view", folderView));
  $effect(() => localStorage.setItem("jc.library.sidebar", collapsed ? "collapsed" : "open"));

  function tell(msg: string) {
    notice = msg;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (notice = null), 2400);
  }

  /** Applies an organisation change from the backend (or shows why not). */
  async function orgDo(p: Promise<Organize>, ok?: string) {
    try {
      org = await p;
      if (ok) tell(ok);
      return true;
    } catch (e) {
      onerror(String(e));
      return false;
    }
  }

  // One scan at a time; a request during a scan runs once after it.
  let again = false;
  async function refresh() {
    if (refreshing) {
      again = true;
      return;
    }
    refreshing = true;
    try {
      const [v, o] = await Promise.all([api.libraryScan(), api.organizeGet().catch((e) => (onerror(`Your folders couldn't be loaded: ${e}`), null))]);
      view = v;
      if (o) org = o;
      loadError = null;
      onscanned?.(clips);
      const ids = new Set(v.clips.map((c) => c.id));
      selection = selection.filter((id) => ids.has(id));
      if (selectedId && !ids.has(selectedId)) selectedId = null;
    } catch (e) {
      loadError = String(e);
    } finally {
      refreshing = false;
      if (again) {
        again = false;
        void refresh();
      }
    }
  }
  let seenToken = untrack(() => refreshToken);
  $effect(() => {
    if (refreshToken !== seenToken) {
      seenToken = refreshToken;
      void refresh();
    }
  });

  onMount(() => {
    void refresh();
    // Pick up clips saved while the window was open (cheap: unchanged files
    // aren't re-read). Only when the window comes back into focus, not on a timer.
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });

  // ---- derived views

  /** Clips with their titles. */
  const clips = $derived((view?.clips ?? []).map((c) => (org.titles[c.key] ? { ...c, title: org.titles[c.key] } : c)));
  const folderOf = $derived.by(() => {
    const m = new Map<string, Folder>();
    for (const f of org.folders) for (const k of f.clips) m.set(k, f);
    return m;
  });
  const currentFolder = $derived(org.folders.find((f) => f.id === folderView) ?? null);
  // A folder deleted elsewhere (or a stale remembered view) falls back to All.
  $effect(() => {
    if (folderView !== "all" && folderView !== "unsorted" && view && !currentFolder) folderView = "all";
  });
  // Folders open in custom order; other views use dates.
  let lastView: View | null = null;
  $effect(() => {
    const v = folderView;
    if (v === lastView) return;
    lastView = v;
    untrack(() => {
      const isFolder = v !== "all" && v !== "unsorted";
      if (isFolder) sort = "custom";
      else if (sort === "custom") sort = "newest";
      selection = [];
      selectedId = null;
    });
  });

  const counts = $derived.by(() => {
    const present = new Set(clips.map((c) => c.key));
    const out: Record<string, number> = {};
    for (const f of org.folders) out[f.id] = f.clips.filter((k) => present.has(k)).length;
    return out;
  });
  const unsortedCount = $derived(clips.filter((c) => !folderOf.has(c.key)).length);

  const inView = $derived.by(() => {
    if (folderView === "all") return clips;
    if (folderView === "unsorted") return clips.filter((c) => !folderOf.has(c.key));
    return clips.filter((c) => folderOf.get(c.key)?.id === folderView);
  });

  const games = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const c of inView) counts.set(c.game, (counts.get(c.game) ?? 0) + 1);
    return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    let list = inView.filter(
      (c) =>
        (!game || c.game === game) &&
        (!q || c.game.toLowerCase().includes(q) || c.file_name.toLowerCase().includes(q) || (c.title ?? "").toLowerCase().includes(q)),
    );
    const dur = (c: Clip) => c.probe?.duration ?? -1;
    if (sort === "custom" && currentFolder) {
      const pos = new Map(currentFolder.clips.map((k, i) => [k, i]));
      list = [...list].sort((a, b) => (pos.get(a.key) ?? 1e9) - (pos.get(b.key) ?? 1e9));
    } else if (sort === "oldest") list = [...list].reverse();
    else if (sort === "longest") list = [...list].sort((a, b) => dur(b) - dur(a));
    else if (sort === "largest") list = [...list].sort((a, b) => b.size - a.size);
    return list;
  });

  // Day groups for date sorts; a single flat group otherwise.
  const groups = $derived.by(() => {
    if (sort !== "newest" && sort !== "oldest") return [{ label: null as string | null, clips: shown }];
    const out: { label: string | null; clips: Clip[] }[] = [];
    for (const c of shown) {
      const label = dayLabel(c.recorded);
      let g = out.at(-1);
      if (g?.label !== label) out.push((g = { label, clips: [] }));
      g!.clips.push(c);
    }
    return out;
  });

  const selectedSet = $derived(new Set(selection));
  const selectedClips = $derived(shown.filter((c) => selectedSet.has(c.id)));
  const primary = $derived(shown.find((c) => c.id === selectedId) ?? selectedClips[0] ?? null);
  // Roving tab stop: the selected card, else the first.
  const tabStop = $derived(primary?.id ?? shown[0]?.id ?? null);

  // ---- selection

  function click(c: Clip, e: MouseEvent) {
    if (e.ctrlKey || e.metaKey) {
      selection = selectedSet.has(c.id) ? selection.filter((id) => id !== c.id) : [...selection, c.id];
      selectedId = c.id;
    } else if (e.shiftKey && selectedId && shown.some((x) => x.id === selectedId)) {
      const a = shown.findIndex((x) => x.id === selectedId);
      const b = shown.findIndex((x) => x.id === c.id);
      const [lo, hi] = a < b ? [a, b] : [b, a];
      selection = shown.slice(lo, hi + 1).map((x) => x.id);
    } else {
      selection = [c.id];
      selectedId = c.id;
    }
  }

  function open(c: Clip) {
    if (c.error) {
      onerror(`This clip can't be read: ${c.error}`);
      return;
    }
    onopen(c, shown);
  }

  /** The clips an action applies to: the selection if `c` is in it, else just `c`. */
  function targetsFor(c: Clip): Clip[] {
    return selectedSet.has(c.id) && selectedClips.length > 1 ? selectedClips : [c];
  }

  // ---- actions

  /** A clip whose file couldn't be read yet has no key: can't be organised. */
  const busyMsg = "That clip couldn't be read just now (in use?). Refresh and try again.";

  async function moveTo(cs: Clip[], folder: Folder | null) {
    if (cs.some((c) => !c.key)) return onerror(busyMsg);
    const keys = cs.map((c) => c.key);
    const what = cs.length === 1 ? `“${cs[0]!.title ?? cs[0]!.game}”` : `${cs.length} clips`;
    await orgDo(api.clipsMove(keys, folder?.id ?? null, null), folder ? `Moved ${what} to ${folder.name}` : `Took ${what} out of its folder`);
  }

  async function saveTitle(c: Clip, title: string | null) {
    renamingId = null;
    if (title === null || title.trim() === (c.title ?? "")) return;
    await orgDo(api.clipTitle(c.key, title));
    await tick();
    cardEls[c.id]?.focus({ preventScroll: true });
  }

  function rename(c: Clip) {
    if (!c.key) return onerror(busyMsg);
    selection = [c.id];
    selectedId = c.id;
    renamingId = c.id;
  }

  async function copyFile(c: Clip) {
    try {
      await api.clipCopyFile(c.id);
      tell("Copied: paste it anywhere with Ctrl+V");
    } catch (e) {
      onerror(String(e));
    }
  }

  function askDelete(cs: Clip[]) {
    const label = cs.length === 1 ? `“${cs[0]!.title ?? cs[0]!.file_name}”` : `${cs.length} clips`;
    dialog = { kind: "delete-clips", ids: cs.map((c) => c.id), label };
  }

  async function doDelete(ids: string[]) {
    dialog = null;
    try {
      const n = await api.clipsDelete(ids);
      tell(n === 1 ? "Moved to the Recycle Bin" : `Moved ${n} clips to the Recycle Bin`);
    } catch (e) {
      onerror(String(e));
    }
    selection = [];
    selectedId = null;
    await refresh();
  }

  function nextColor(): FolderColor {
    const used = new Set(org.folders.map((f) => f.color));
    return FOLDER_COLORS.find((c) => !used.has(c.value))?.value ?? "gray";
  }

  async function createFolder(name: string, moveKeys: string[]) {
    const before = new Set(org.folders.map((f) => f.id));
    try {
      org = await api.folderCreate(name, nextColor());
    } catch (e) {
      if (dialog?.kind === "new-folder") dialog = { ...dialog, error: String(e) };
      return;
    }
    dialog = null;
    const f = org.folders.find((x) => !before.has(x.id));
    if (f && moveKeys.length) await orgDo(api.clipsMove(moveKeys, f.id, null), `Moved to ${f.name}`);
    else if (f) tell(`Created ${f.name}: drag clips onto it`);
  }

  async function renameFolder(f: Folder, name: string) {
    try {
      org = await api.folderRename(f.id, name);
      dialog = null;
    } catch (e) {
      if (dialog?.kind === "rename-folder") dialog = { ...dialog, error: String(e) };
    }
  }

  async function deleteFolder(f: Folder) {
    dialog = null;
    if (folderView === f.id) folderView = "all";
    await orgDo(api.folderDelete(f.id), `Deleted the folder ${f.name}. Its clips are still in All clips.`);
  }

  // ---- menus

  function moveMenu(cs: Clip[]): MenuItem[] {
    const here = cs.length === 1 ? folderOf.get(cs[0]!.key)?.id : undefined;
    const items: MenuItem[] = org.folders.map((f) => ({ label: f.name, dot: f.color, checked: f.id === here, action: () => moveTo(cs, f) }));
    items.push({ label: "Not in a folder", checked: cs.length === 1 && !here, action: () => moveTo(cs, null) });
    items.push({ label: "New folder…", action: () => (dialog = { kind: "new-folder", moveKeys: cs.map((c) => c.key), error: null }) });
    return items;
  }

  function clipMenu(e: MouseEvent, c: Clip) {
    if (!selectedSet.has(c.id)) {
      selection = [c.id];
      selectedId = c.id;
    }
    const cs = targetsFor(c);
    const many = cs.length > 1;
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "Open", shortcut: "Enter", disabled: many || !!c.error, action: () => open(c) },
        { label: "Rename", shortcut: "F2", disabled: many, action: () => rename(c) },
        { label: many ? `Move ${cs.length} clips to` : "Move to folder", submenu: moveMenu(cs) },
        "separator",
        { label: "Show in folder", disabled: many, action: () => api.showInFolder(c.id).catch((err) => onerror(String(err))) },
        { label: "Copy file", disabled: many, action: () => copyFile(c) },
        { label: "Open in default player", disabled: many, action: () => api.openExternal(c.id).catch((err) => onerror(String(err))) },
        "separator",
        { label: many ? `Delete ${cs.length} clips…` : "Delete…", shortcut: "Del", danger: true, action: () => askDelete(cs) },
      ],
    };
  }

  function folderMenu(e: MouseEvent, f: Folder) {
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "Open", action: () => (folderView = f.id) },
        { label: "Rename…", action: () => (dialog = { kind: "rename-folder", folder: f, error: null }) },
        {
          label: "Color",
          submenu: FOLDER_COLORS.map((c) => ({ label: c.label, dot: c.value, checked: f.color === c.value, action: () => orgDo(api.folderColor(f.id, c.value)) })),
        },
        "separator",
        { label: "Delete folder…", danger: true, action: () => (dialog = { kind: "delete-folder", folder: f, count: counts[f.id] ?? 0 }) },
      ],
    };
  }

  // ---- drag and drop

  const reorderBlocked = (): string | null => {
    if (!currentFolder) return "Open a folder to arrange clips in your own order";
    if (sort !== "custom") return "Switch the sort to Custom order to arrange clips";
    if (query.trim() || game) return "Clear the search and game filter to arrange clips";
    return null;
  };

  function cardDown(e: PointerEvent, c: Clip) {
    startDrag(
      e,
      (): Payload => {
        if (!selectedSet.has(c.id)) {
          selection = [c.id];
          selectedId = c.id;
        }
        const cs = targetsFor(c);
        return {
          kind: "clips",
          ids: cs.map((x) => x.id),
          keys: cs.map((x) => x.key),
          label: cs.length > 1 ? `${cs.length} clips` : (c.title ?? c.game),
          thumb: c.error ? null : thumbUrl(c.id),
        };
      },
      (t: Target, p: Payload) => {
        if (p.kind !== "clips") return;
        const cs = clips.filter((x) => p.ids.includes(x.id));
        if (t.type === "folder") {
          const f = t.id ? (org.folders.find((x) => x.id === t.id) ?? null) : null;
          if (cs.every((x) => (folderOf.get(x.key)?.id ?? null) === (f?.id ?? null))) return;
          void moveTo(cs, f);
        } else if (t.type === "insert" && currentFolder) {
          void orgDo(api.clipsMove(p.keys, currentFolder.id, t.before));
        }
      },
      {
        reorder: {
          cards: () => shown.map((x) => ({ key: x.key, el: cardEls[x.id]! })).filter((x) => x.el),
          area: () => gridEl,
          blocked: reorderBlocked,
        },
      },
    );
  }

  const dragging = $derived(drag.payload?.kind === "clips" ? new Set(drag.payload.ids) : null);

  // ---- keyboard

  /** Columns in the grid right now (for up/down). */
  function columns(): number {
    const cards = shown.map((c) => cardEls[c.id]).filter(Boolean) as HTMLElement[];
    const first = cards[0];
    if (!first || cards.length < 2) return 1;
    const top = first.offsetTop;
    const n = cards.findIndex((el) => el.offsetTop !== top);
    return n === -1 ? cards.length : n;
  }

  async function onkey(e: KeyboardEvent) {
    if (!shown.length || fromWithin(e, "input, select") || renamingId) return;
    const i = Math.max(0, shown.findIndex((c) => c.id === selectedId));
    let next = -1;
    switch (e.key) {
      case "ArrowRight":
        next = i + 1;
        break;
      case "ArrowLeft":
        next = i - 1;
        break;
      case "ArrowDown":
        next = i + columns();
        break;
      case "ArrowUp":
        next = i - columns();
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = shown.length - 1;
        break;
      case "Enter":
        if (primary) {
          e.preventDefault();
          open(primary);
        }
        return;
      case "F2":
        if (primary) {
          e.preventDefault();
          rename(primary);
        }
        return;
      case "Delete":
        if (selectedClips.length) {
          e.preventDefault();
          askDelete(selectedClips);
        }
        return;
      case "a":
      case "A":
        if (e.ctrlKey) {
          e.preventDefault();
          selection = shown.map((c) => c.id);
        }
        return;
      case "ContextMenu":
      case "F10":
        if (primary && (e.key === "ContextMenu" || e.shiftKey)) {
          e.preventDefault();
          const r = cardEls[primary.id]?.getBoundingClientRect();
          if (r) clipMenu(new MouseEvent("contextmenu", { clientX: r.left + 24, clientY: r.top + 24 }), primary);
        }
        return;
      case "Escape":
        selection = [];
        selectedId = null;
        return;
      default:
        return;
    }
    e.preventDefault();
    next = Math.min(shown.length - 1, Math.max(0, next));
    const target = shown[next];
    if (!target) return;
    if (e.shiftKey && selectedId && shown.some((x) => x.id === selectedId)) {
      const a = shown.findIndex((x) => x.id === selectedId);
      const [lo, hi] = a < next ? [a, next] : [next, a];
      selection = shown.slice(lo, hi + 1).map((x) => x.id);
    } else {
      selection = [target.id];
      selectedId = target.id;
    }
    await tick();
    const el = cardEls[target.id];
    el?.focus({ preventScroll: true });
    el?.scrollIntoView({ block: "nearest" });
  }

  const sorts = $derived([
    ...(currentFolder ? [{ value: "custom", label: "Custom order" }] : []),
    { value: "newest", label: "Newest first" },
    { value: "oldest", label: "Oldest first" },
    { value: "longest", label: "Longest first" },
    { value: "largest", label: "Largest first" },
  ]);
  const total = $derived(shown.reduce((n, c) => n + c.size, 0));
  const viewName = $derived(folderView === "all" ? "All clips" : folderView === "unsorted" ? "Not in a folder" : (currentFolder?.name ?? ""));
</script>

<div class="lib">
  <FolderSidebar
    folders={org.folders}
    {counts}
    allCount={clips.length}
    {unsortedCount}
    view={folderView}
    {collapsed}
    onview={(v) => (folderView = v)}
    oncreate={() => (dialog = { kind: "new-folder", moveKeys: [], error: null })}
    oncontext={folderMenu}
    onreorder={(ids) => orgDo(api.foldersReorder(ids))}
    ontoggle={() => (collapsed = !collapsed)}
  />

  <div class="main">
  {#if !view || view.clips.length || loadError}
  <div class="toolbar">
    <div class="chips" role="group" aria-label="Filter by game">
      {#if currentFolder}
        <span class="where" title={currentFolder.name}><span class="fdot" style:background="var(--folder-{currentFolder.color})"></span>{currentFolder.name}</span>
      {/if}
      <button type="button" class="chip" class:on={!game} aria-pressed={!game} onclick={() => (game = "")}>
        All games <span class="n">{inView.length}</span>
      </button>
      {#each games as [name, n] (name)}
        <button type="button" class="chip" class:on={game === name} aria-pressed={game === name} onclick={() => (game = game === name ? "" : name)}>
          {name} <span class="n">{n}</span>
        </button>
      {/each}
    </div>
    <div class="tools">
      <label class="search">
        <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.5" /><path d="m10.5 10.5 3 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /></svg>
        <input type="search" placeholder="Search clips" aria-label="Search clips" bind:value={query} spellcheck="false" />
      </label>
      <Select label="Sort clips" value={sort} options={sorts} onchange={(v) => (sort = v as Sort)} />
      <button type="button" class="icon-btn" title="Refresh" aria-label="Refresh library" onclick={refresh} disabled={refreshing}>
        <svg class:spin={refreshing} viewBox="0 0 16 16" aria-hidden="true"><path d="M13 8a5 5 0 1 1-1.6-3.7M13 2.5v2.8h-2.8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
    </div>
  </div>
  {/if}

  {#if loadError}
    <div class="pad">
      <Banner tone="error">
        Couldn't read the clips folder: {loadError}
        {#snippet action()}<Button onclick={refresh}>Try again</Button>{/snippet}
      </Banner>
    </div>
  {:else if !view}
    <div class="grid pad" aria-busy="true" aria-label="Loading clips">
      {#each Array(8) as _, i (i)}<div class="sk"><div class="sk-thumb"></div><div class="sk-line"></div></div>{/each}
    </div>
  {:else if !view.clips.length}
    <div class="empty">
      <svg viewBox="0 0 48 48" aria-hidden="true"><rect x="6" y="11" width="36" height="26" rx="5" fill="none" stroke="currentColor" stroke-width="2" /><circle cx="24" cy="24" r="6" fill="var(--accent)" /></svg>
      <h2>No clips yet</h2>
      <p>Press {#if hotkey}<kbd>{hotkey}</kbd>{:else}your save hotkey{/if} while playing to save the last few minutes. Your clips show up here.</p>
      <Button onclick={() => api.openFolder(view?.root ?? "")}>Open clips folder</Button>
    </div>
  {:else if !inView.length && currentFolder}
    <div class="empty">
      <svg viewBox="0 0 48 48" aria-hidden="true" style:color="var(--folder-{currentFolder.color})"><path d="M6 14a3 3 0 0 1 3-3h10l4 4.5h16a3 3 0 0 1 3 3V35a3 3 0 0 1-3 3H9a3 3 0 0 1-3-3z" fill="currentColor" fill-opacity="0.15" stroke="currentColor" stroke-width="2" stroke-linejoin="round" /></svg>
      <h2>{currentFolder.name} is empty</h2>
      <p>Drag clips from All clips onto this folder in the sidebar, or right-click a clip and choose Move to folder.</p>
      <Button onclick={() => (folderView = "all")}>Go to All clips</Button>
    </div>
  {:else if !inView.length}
    <div class="empty">
      <h2>Every clip is in a folder</h2>
      <p>Clips you haven't sorted into a folder show up here.</p>
    </div>
  {:else if !shown.length}
    <div class="empty">
      <h2>No clips match</h2>
      <p>Nothing matches “{query}”{game ? ` in ${game}` : ""}{folderView !== "all" ? ` in ${viewName}` : ""}.</p>
      <Button onclick={() => ((query = ""), (game = ""))}>Clear filters</Button>
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="scroll pad" bind:this={gridEl} onkeydown={onkey}>
      {#each groups as g (g.label)}
        <section class="group" class:flat={!g.label} aria-label={g.label ?? viewName}>
          {#if g.label}
            <h2>{g.label} <span>{g.clips.length} {g.clips.length === 1 ? "clip" : "clips"}</span></h2>
          {/if}
          <div class="grid">
            {#each g.clips as c (c.id)}
              <ClipCard
                clip={c}
                selected={selectedSet.has(c.id)}
                tabindex={c.id === tabStop ? 0 : -1}
                folderColor={folderView === "all" ? (folderOf.get(c.key)?.color ?? null) : null}
                renaming={renamingId === c.id}
                lifted={dragging?.has(c.id) ?? false}
                onselect={(e) => click(c, e)}
                onopen={() => open(c)}
                oncontext={(e) => clipMenu(e, c)}
                onpointerdown={(e) => cardDown(e, c)}
                onrename={(t) => saveTitle(c, t)}
                bind:el={cardEls[c.id]}
              />
            {/each}
          </div>
        </section>
      {/each}
      <p class="foot">{shown.length} {shown.length === 1 ? "clip" : "clips"} · {bytes(total)} · <button type="button" class="link" onclick={() => api.openFolder(view?.root ?? "")}>{view.root}</button></p>
    </div>
  {/if}

  {#if selectedClips.length > 1}
    <div class="selbar" role="region" aria-label="Selected clips">
      <div class="sel-meta">
        <strong>{selectedClips.length} clips selected</strong>
        <span>{bytes(selectedClips.reduce((n, c) => n + c.size, 0))} · drag them onto a folder, or</span>
      </div>
      <div class="sel-actions">
        <Button variant="ghost" onclick={() => ((selection = []), (selectedId = null))}>Clear</Button>
        <Button variant="ghost" onclick={() => askDelete(selectedClips)}>Delete…</Button>
        <Button
          variant="primary"
          onclick={(e) => {
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            menu = { x: r.left, y: r.top - 8 - 32 * (org.folders.length + 2), items: moveMenu(selectedClips) };
          }}
        >
          Move to folder
        </Button>
      </div>
    </div>
  {:else if primary}
    {@const selected = primary}
    <div class="selbar" role="region" aria-label="Selected clip">
      <div class="sel-thumb">
        {#if !selected.error}<img src={thumbUrl(selected.id)} alt="" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.visibility = "hidden")} />{/if}
      </div>
      <div class="sel-meta">
        <strong title={selected.title ?? selected.game}>{selected.title ?? selected.game}</strong>
        <span>{#if selected.title}{selected.game} · {/if}{whenShort(selected.recorded)} · {selected.probe ? clock(selected.probe.duration) : "unreadable"} · {bytes(selected.size)}{#if folderOf.get(selected.key)} · {folderOf.get(selected.key)?.name}{/if}</span>
      </div>
      <div class="sel-actions">
        <Button variant="ghost" onclick={() => rename(selected)} title="Rename (F2)">Rename</Button>
        <Button variant="ghost" onclick={() => api.showInFolder(selected.id).catch((e) => onerror(String(e)))}>Show in folder</Button>
        <Button variant="ghost" onclick={() => api.openExternal(selected.id).catch((e) => onerror(String(e)))}>Open in player</Button>
        <Button variant="primary" onclick={() => open(selected)} disabled={!!selected.error}>
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M5 3.5v9l7-4.5z" fill="currentColor" /></svg>
          Open
        </Button>
      </div>
    </div>
  {/if}
  </div>
</div>

<DragGhost />
{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}
{#if notice}<div class="notice" role="status">{notice}</div>{/if}
{#if dialog?.kind === "new-folder"}
  {@const d = dialog}
  <Dialog
    title="New folder"
    confirmLabel={d.moveKeys.length ? "Create and move" : "Create"}
    input={{ value: "", placeholder: "e.g. DayZ Raids", max: 60 }}
    error={d.error}
    onconfirm={(v) => createFolder(v, d.moveKeys)}
    oncancel={() => (dialog = null)}
  />
{:else if dialog?.kind === "rename-folder"}
  {@const d = dialog}
  <Dialog title="Rename folder" confirmLabel="Rename" input={{ value: d.folder.name, max: 60 }} error={d.error} onconfirm={(v) => renameFolder(d.folder, v)} oncancel={() => (dialog = null)} />
{:else if dialog?.kind === "delete-folder"}
  {@const d = dialog}
  <Dialog title="Delete “{d.folder.name}”?" confirmLabel="Delete folder" danger onconfirm={() => deleteFolder(d.folder)} oncancel={() => (dialog = null)}>
    <p>Only the folder is removed. {#if d.count}Its {d.count === 1 ? "clip stays" : `${d.count} clips stay`} in your library under All clips.{:else}It's empty.{/if} No recordings are deleted.</p>
  </Dialog>
{:else if dialog?.kind === "delete-clips"}
  {@const d = dialog}
  <Dialog title="Delete {d.label}?" confirmLabel="Move to Recycle Bin" danger onconfirm={() => doDelete(d.ids)} oncancel={() => (dialog = null)}>
    <p>{d.ids.length === 1 ? "The recording is" : "The recordings are"} moved to the Windows Recycle Bin, so you can restore {d.ids.length === 1 ? "it" : "them"} from there. Titles and folders come back if you do.</p>
  </Dialog>
{/if}

<style>
  .lib {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .where {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    max-width: 220px;
    margin-right: var(--s1);
    font-size: var(--fs-md);
    font-weight: var(--fw-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: none;
  }
  .fdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .group.flat {
    padding-top: var(--s4);
  }
  .notice {
    position: fixed;
    left: 50%;
    bottom: var(--s7);
    transform: translateX(-50%);
    z-index: 55;
    max-width: calc(100vw - 48px);
    padding: var(--s2) var(--s4);
    border-radius: var(--radius-pill);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-sm);
  }
  :global(body.dragging),
  :global(body.dragging *) {
    cursor: grabbing !important;
    user-select: none;
  }
  .pad {
    padding: 0 var(--s5);
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s4);
    padding: var(--s3) var(--s5);
    border-bottom: 1px solid var(--border);
  }
  .chips {
    display: flex;
    gap: var(--s2);
    overflow-x: auto;
    min-width: 0;
    scrollbar-width: none;
  }
  .chip {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    height: 30px;
    padding: 0 var(--s3);
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease);
  }
  .chip:hover {
    background: var(--surface-hover);
    color: var(--text);
  }
  .chip.on {
    background: var(--text);
    border-color: var(--text);
    color: var(--bg);
    font-weight: var(--fw-strong);
  }
  .chip .n {
    font-size: var(--fs-xs);
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: var(--s2);
    flex: none;
  }
  .tools :global(select) {
    width: 148px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--s2);
    height: var(--control-h);
    width: 200px;
    padding: 0 var(--s3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--surface);
    color: var(--text-faint);
  }
  .search:focus-within {
    border-color: var(--text-faint);
  }
  .search svg {
    width: 14px;
    height: 14px;
    flex: none;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    font-size: var(--fs-sm);
    user-select: text;
  }
  .search input::placeholder {
    color: var(--text-faint);
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: var(--control-h);
    height: var(--control-h);
    border-radius: var(--radius-md);
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text);
  }
  .icon-btn svg {
    width: 16px;
    height: 16px;
  }
  .spin {
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: var(--s5);
  }
  .group {
    content-visibility: auto;
    contain-intrinsic-size: auto 600px;
  }
  .group h2 {
    position: sticky;
    top: 0;
    z-index: 1;
    margin: 0;
    padding: var(--s4) 0 var(--s3);
    background: var(--bg);
    font-size: var(--fs-md);
    font-weight: var(--fw-strong);
  }
  .group h2 span {
    margin-left: var(--s2);
    color: var(--text-faint);
    font-size: var(--fs-xs);
    font-weight: var(--fw-regular);
  }
  .group:first-child:not(:has(h2)) {
    padding-top: var(--s4);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(232px, 1fr));
    gap: var(--s4);
  }
  .grid.pad {
    padding-top: var(--s5);
  }
  .foot {
    margin: var(--s5) 0 0;
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font-size: inherit;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link:hover {
    color: var(--text-muted);
  }
  .sk {
    border-radius: var(--radius-lg);
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .sk-thumb {
    aspect-ratio: 16 / 9;
    background: linear-gradient(90deg, var(--surface) 0%, var(--surface-raised) 50%, var(--surface) 100%);
    background-size: 200% 100%;
    animation: shimmer 1.2s linear infinite;
  }
  .sk-line {
    height: 44px;
    background: var(--surface);
  }
  @keyframes shimmer {
    to {
      background-position: -200% 0;
    }
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s3);
    padding: var(--s7) var(--s5);
    text-align: center;
    color: var(--text-muted);
  }
  .empty svg {
    width: 56px;
    height: 56px;
    color: var(--text-faint);
  }
  .empty h2 {
    margin: 0;
    color: var(--text);
    font-size: var(--fs-xl);
    font-family: var(--font-display);
    font-weight: var(--fw-strong);
  }
  .empty p {
    margin: 0 0 var(--s2);
    max-width: 360px;
    font-size: var(--fs-sm);
  }
  kbd {
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-strong);
    background: var(--surface-raised);
    color: var(--text);
    font-family: var(--font);
    font-size: var(--fs-xs);
    font-weight: var(--fw-strong);
  }
  .selbar {
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: var(--s3) var(--s5);
    border-top: 1px solid var(--border);
    background: var(--surface);
  }
  .sel-thumb {
    width: 64px;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-sm);
    background: var(--surface-raised);
    overflow: hidden;
    flex: none;
  }
  .sel-thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .sel-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
    font-size: var(--fs-sm);
  }
  .sel-meta span {
    color: var(--text-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sel-actions {
    display: flex;
    align-items: center;
    gap: var(--s1);
    flex: none;
  }
</style>
