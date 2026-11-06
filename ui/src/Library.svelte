<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { api, FOLDER_COLORS, MAX_DEPTH, isWithin, thumbUrl, type Clip, type ExportEvent, type Folder, type FolderColor, type Item, type LibraryView, type MoveResult, type PendingFolder } from "./lib/api";
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

  type Sort = "custom" | "newest" | "oldest" | "name" | "longest" | "largest";
  let view = $state<LibraryView | null>(null);
  // Folders are real directories (ADR-0015); this mirrors the last load.
  let org = $state<{ folders: Folder[]; pending: PendingFolder[] }>({ folders: [], pending: [] });
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
    | { kind: "new-folder"; moveIds: string[]; parent: string | null; color: FolderColor; error: string | null }
    | { kind: "rename-folder"; folder: Folder; error: string | null }
    | { kind: "delete-folder"; folder: Folder; count: number; subs: number }
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

  function tell(msg: string, ms = 2400) {
    notice = msg;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (notice = null), ms);
  }

  // ---- Share to Discord / Save to Drive from the menu (whole clip)

  /** The running share or Drive copy started here (the editor runs its own). */
  let job = $state<{ kind: "share" | "drive"; clip: Clip; id: number | null; progress: number } | null>(null);
  /** The Drive folder setting (null: off), for the menu. */
  let driveDir = $state<string | null>(null);
  const loadDrive = () =>
    api
      .driveStatus()
      .then((d) => (driveDir = d.dir || null))
      .catch(() => {});
  const nameOf = (c: Clip) => c.title ?? c.game;

  async function shareClip(c: Clip) {
    if (job || !c.probe) return;
    try {
      const plan = await api.sharePlan(c.id, 0, c.probe.duration);
      if (!plan.plan) {
        onerror(`${nameOf(c)} is too long for ${plan.limit_mb} MB. Open it and trim it to under ${clock(plan.max_seconds ?? 0)}.`);
        return;
      }
      job = { kind: "share", clip: c, id: null, progress: 0 };
      const r = await api.shareStart(c.id, 0, c.probe.duration, c.title ?? null);
      if (job) job = { ...job, id: r.job };
    } catch (e) {
      job = null;
      onerror(String(e));
    }
  }

  async function saveClipToDrive(c: Clip) {
    if (job || !c.probe) return;
    job = { kind: "drive", clip: c, id: null, progress: 0 };
    try {
      const r = await api.driveSave(c.id, 0, c.probe.duration, true, false, c.title ?? null);
      if (r.already || r.job === null) {
        job = null;
        await api.clipDriveMark(c.key, r.output).catch(() => {});
        tell(`${nameOf(c)} is already in your Google Drive folder`, 4000);
        await refresh();
      } else if (job) job = { ...job, id: r.job };
    } catch (e) {
      job = null;
      onerror(String(e));
    }
  }

  async function onJobEvent(e: ExportEvent) {
    const j = job;
    if (!j || (j.id !== null && e.job !== j.id)) return;
    if (e.kind === "progress") job = { ...j, progress: e.progress };
    else if (e.kind === "done") {
      job = null;
      if (j.kind === "share") {
        const err = e.clipboard && "Err" in e.clipboard ? e.clipboard.Err : null;
        if (err) onerror(`The Discord copy of ${nameOf(j.clip)} is ready but couldn't go on the clipboard: ${err}`);
        else tell(`Copied ${nameOf(j.clip)}: paste it in Discord with Ctrl+V · ${e.share ? `${e.share.height}p${e.share.fps} · ` : ""}${bytes(e.size)}`, 6000);
      } else {
        await api.clipDriveMark(j.clip.key, e.path).catch(() => {});
        tell(`Saved ${nameOf(j.clip)} in your Google Drive folder · uploads when Google Drive for desktop syncs`, 6000);
        await refresh();
      }
    } else if (e.kind === "error") {
      job = null;
      onerror(e.message);
    } else job = null; // cancelled
  }

  /** Runs an organisation change, then reloads from disk (or shows why not). */
  async function orgDo(p: Promise<unknown>, ok?: string) {
    try {
      await p;
      if (ok) tell(ok);
      return true;
    } catch (e) {
      onerror(String(e));
      return false;
    } finally {
      await refresh();
    }
  }

  /** Reports a file move: what moved, and what couldn't (nothing is lost). */
  function moved(r: MoveResult, where: string) {
    if (r.failed.length) onerror(`${r.moved ? `Moved ${r.moved}, but ` : ""}${r.failed.length === 1 ? "1 clip" : `${r.failed.length} clips`} couldn't be moved: ${r.failed[0]}`);
    else if (r.moved) tell(r.moved === 1 ? `Moved to ${where}` : `Moved ${r.moved} clips to ${where}`);
  }

  // One scan at a time; a request during a scan runs once after it.
  let again = false;
  let afterLoad: (() => void) | null = null;
  async function refresh() {
    if (refreshing) {
      again = true;
      return;
    }
    refreshing = true;
    try {
      // A move changes a clip's path (its id): keep the selection by content key.
      const keyOf = new Map((view?.clips ?? []).map((c) => [c.id, c.key]));
      const r = await api.libraryLoad();
      view = r.view;
      org = { folders: r.folders, pending: r.pending };
      afterLoad?.();
      afterLoad = null;
      loadError = r.rootOk ? null : `The clips folder ${r.view.root} can't be reached. Is the drive connected?`;
      onscanned?.(clips);
      const byKey = new Map(r.view.clips.filter((c) => c.key).map((c) => [c.key, c.id]));
      const ids = new Set(r.view.clips.map((c) => c.id));
      const follow = (id: string) => (ids.has(id) ? id : (byKey.get(keyOf.get(id) ?? "") ?? null));
      selection = selection.map(follow).filter((id): id is string => !!id);
      selectedId = selectedId ? follow(selectedId) : null;
      if (renamingId) renamingId = follow(renamingId);
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
    void loadDrive();
    let unlisten: (() => void) | undefined;
    let gone = false;
    void api.onExport(onJobEvent).then((u) => (gone ? u() : (unlisten = u)));
    // Pick up clips saved while the window was open (cheap: unchanged files
    // aren't re-read). Only when the window comes back into focus, not on a timer.
    const onFocus = () => {
      void refresh();
      void loadDrive();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      gone = true;
      unlisten?.();
      window.removeEventListener("focus", onFocus);
    };
  });

  // ---- derived views

  const clips = $derived(view?.clips ?? []);
  const byName = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
  /** clip id → the folder its file is in */
  const folderOf = $derived.by(() => {
    const byName = new Map(org.folders.map((f) => [f.name, f]));
    const m = new Map<string, Folder>();
    for (const c of clips) {
      const f = c.folder ? byName.get(c.folder) : undefined;
      if (f) m.set(c.id, f);
    }
    return m;
  });
  const currentFolder = $derived(org.folders.find((f) => f.id === folderView) ?? null);
  // A folder deleted elsewhere (or a stale remembered view) falls back to All.
  $effect(() => {
    if (folderView !== "all" && folderView !== "unsorted" && view && !currentFolder) folderView = "all";
  });
  // Each view remembers its sort (a UI preference). Folders start in custom
  // order, the other views by date. Sorting never changes the custom order.
  const sortMemory: Record<string, Sort> = (() => {
    try {
      return JSON.parse(localStorage.getItem("jc.library.sort") ?? "{}") as Record<string, Sort>;
    } catch {
      return {};
    }
  })();
  let lastView: View | null = null;
  $effect(() => {
    const v = folderView;
    if (v === lastView) return;
    lastView = v;
    untrack(() => {
      const isFolder = v !== "all" && v !== "unsorted";
      const kept = sortMemory[v];
      sort = kept && (isFolder || kept !== "custom") ? kept : isFolder ? "custom" : "newest";
      selection = [];
      selectedId = null;
    });
  });
  function setSort(v: Sort) {
    sort = v;
    sortMemory[folderView] = v;
    try {
      localStorage.setItem("jc.library.sort", JSON.stringify(sortMemory));
    } catch {
      // not remembered: fine
    }
  }

  /** Clips directly in each folder. */
  const own = $derived.by(() => {
    const out: Record<string, number> = {};
    for (const f of folderOf.values()) out[f.id] = (out[f.id] ?? 0) + 1;
    return out;
  });
  /** Sidebar counts: a folder's clips including its subfolders. */
  const counts = $derived.by(() => {
    const out: Record<string, number> = {};
    for (const f of org.folders) out[f.id] = org.folders.filter((x) => x.id === f.id || isWithin(x.id, f.id)).reduce((n, x) => n + (own[x.id] ?? 0), 0);
    return out;
  });
  const unsortedCount = $derived(clips.filter((c) => !folderOf.has(c.id)).length);
  const hasSubfolders = $derived(!!currentFolder && org.folders.some((x) => x.parent === currentFolder.id));

  // Show a folder's subfolder clips too (a UI preference, off by default:
  // like Explorer, a folder shows what's directly in it).
  let withSubs = $state(localStorage.getItem("jc.library.subs") === "on");
  $effect(() => localStorage.setItem("jc.library.subs", withSubs ? "on" : "off"));
  const showingSubs = $derived(withSubs && hasSubfolders);

  const inView = $derived.by(() => {
    if (folderView === "all") return clips;
    if (folderView === "unsorted") return clips.filter((c) => !folderOf.has(c.id));
    if (showingSubs) return clips.filter((c) => { const id = folderOf.get(c.id)?.id; return !!id && (id === folderView || isWithin(id, folderView)); });
    return clips.filter((c) => folderOf.get(c.id)?.id === folderView);
  });

  /** Folders in tree order (parents first), for menus. */
  const tree = $derived.by(() => {
    const out: Folder[] = [];
    const add = (parent: string | null) => {
      for (const f of org.folders) if (f.parent === parent) (out.push(f), add(f.id));
    };
    add(null);
    for (const f of org.folders) if (!out.includes(f)) out.push(f);
    return out;
  });
  const pathLabel = (id: string) => id.replaceAll("\\", " › ");

  /** After a folder is renamed or moved, keep showing it (or the subfolder
   *  inside it that was open). Runs inside the reload, before the "folder
   *  gone" fallback can see the old path. */
  function follow(from: string, to: string) {
    const v = folderView;
    if (v !== from && !isWithin(v, from)) return;
    const want = (to + v.slice(from.length)).toLowerCase();
    afterLoad = () => {
      const f = org.folders.find((x) => x.id.toLowerCase() === want);
      if (f) folderView = f.id;
    };
  }

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
    else if (sort === "name") list = [...list].sort((a, b) => byName.compare(a.title ?? a.game, b.title ?? b.game) || b.recorded.localeCompare(a.recorded));
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

  const items = (cs: Clip[]): Item[] => cs.map((c) => ({ id: c.id, key: c.key }));

  /** Moves the files into `folder` (null: back to their game folders). */
  async function moveTo(cs: Clip[], folder: Folder | null) {
    if (cs.some((c) => !c.key)) return onerror(busyMsg);
    const todo = cs.filter((c) => (folderOf.get(c.id)?.id ?? null) !== (folder?.id ?? null));
    if (!todo.length) return;
    try {
      moved(await api.clipsMove(items(todo), folder?.name ?? null), folder?.name ?? "its game folder");
    } catch (e) {
      onerror(String(e));
    }
    await refresh();
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

  /** Opens the New folder dialog; the colour starts as the parent's, or the next unused one. */
  function newFolder(moveIds: string[], parent: string | null) {
    const color = (parent && org.folders.find((x) => x.id === parent)?.color) || nextColor();
    dialog = { kind: "new-folder", moveIds, parent, color, error: null };
  }

  async function createFolder(name: string, color: FolderColor, moveIds: string[], parent: string | null) {
    const before = new Set(org.folders.map((f) => f.id));
    try {
      await api.folderCreate(name, color, parent);
    } catch (e) {
      if (dialog?.kind === "new-folder") dialog = { ...dialog, error: String(e) };
      return;
    }
    dialog = null;
    await refresh();
    const f = org.folders.find((x) => !before.has(x.id));
    const cs = clips.filter((c) => moveIds.includes(c.id));
    if (f && cs.length) await moveTo(cs, f);
    else if (f) tell(`Created ${f.leaf}: drag clips onto it`);
  }

  async function renameFolder(f: Folder, name: string) {
    try {
      await api.folderRename(f.name, name);
      dialog = null;
    } catch (e) {
      if (dialog?.kind === "rename-folder") dialog = { ...dialog, error: String(e) };
      return;
    }
    const leaf = name.trim().replace(/\s+/g, " ");
    follow(f.id, f.parent ? `${f.parent}\\${leaf}` : leaf);
    await refresh();
  }

  /** Puts a folder inside `parent` (null: top level), before sibling
   *  `before` (null: last). Moving changes its directory on disk; the order
   *  is metadata. */
  async function placeFolder(id: string, parent: string | null, before: string | null) {
    const f = org.folders.find((x) => x.id === id);
    if (!f) return;
    let cur = id;
    if ((f.parent ?? "").toLowerCase() !== (parent ?? "").toLowerCase()) {
      try {
        await api.folderMove(id, parent);
      } catch (e) {
        onerror(String(e));
        return;
      }
      cur = parent ? `${parent}\\${f.leaf}` : f.leaf;
      follow(id, cur);
      await refresh();
      tell(parent ? `Moved ${f.leaf} into ${org.folders.find((x) => x.id === parent)?.leaf ?? parent}` : `Moved ${f.leaf} to the top level`);
    }
    const ids = org.folders.map((x) => x.id).filter((x) => x.toLowerCase() !== cur.toLowerCase());
    const at = before ? ids.indexOf(before) : -1;
    const real = org.folders.find((x) => x.id.toLowerCase() === cur.toLowerCase())?.id ?? cur;
    if (at >= 0) ids.splice(at, 0, real);
    else ids.push(real);
    await orgDo(api.foldersReorder(ids));
  }

  async function deleteFolder(f: Folder) {
    dialog = null;
    if (folderView === f.id || isWithin(folderView, f.id)) folderView = f.parent ?? "all";
    try {
      const r = await api.folderDelete(f.name);
      if (r.failed.length) onerror(`${r.failed.length === 1 ? "1 clip" : `${r.failed.length} clips`} couldn't be moved out, so the folder ${f.leaf} was kept: ${r.failed[0]}`);
      else if (r.kept_dir && r.kept_reason === "in-use")
        onerror(`${r.moved ? `Moved ${r.moved === 1 ? "its clip" : `its ${r.moved} clips`} out, but the` : "The"} empty folder ${f.leaf} couldn't be removed: another program has it open (a File Explorer window?). Close it and delete the folder again.`);
      else if (r.kept_dir)
        onerror(`${r.moved ? `Moved ${r.moved === 1 ? "its clip" : `its ${r.moved} clips`} out, but` : ""} ${f.leaf} still has other files (${r.kept_files.join(", ")}), so its Windows folder was kept.`.trim());
      else tell(r.moved ? `Deleted ${f.leaf}. Its ${r.moved === 1 ? "clip is" : `${r.moved} clips are`} back in their game folders.` : `Deleted ${f.leaf}`);
    } catch (e) {
      onerror(String(e));
    }
    await refresh();
  }

  /** Clips of a folder from before real folders (ADR-0015): move them in, or leave them. */
  async function resolvePending(p: PendingFolder, moveThem: boolean) {
    const cs = clips.filter((c) => p.ids.includes(c.id));
    try {
      const r = await api.pendingResolve(p.folder, items(cs), moveThem);
      if (moveThem) moved(r, p.folder);
    } catch (e) {
      onerror(String(e));
    }
    await refresh();
  }

  // ---- menus

  function moveMenu(cs: Clip[]): MenuItem[] {
    const here = cs.length === 1 ? folderOf.get(cs[0]!.id)?.id : undefined;
    const items: MenuItem[] = tree.map((f) => ({ label: pathLabel(f.id), dot: f.color, checked: f.id === here, action: () => moveTo(cs, f) }));
    items.push({ label: "Not in a folder", checked: cs.length === 1 && !here, action: () => moveTo(cs, null) });
    items.push({ label: "New folder…", action: () => newFolder(cs.map((c) => c.id), null) });
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
        { label: "Share to Discord", disabled: many || !!c.error || !!job, action: () => void shareClip(c) },
        ...(driveDir
          ? [
              c.in_drive
                ? { label: "Show in Drive", disabled: many, action: () => api.driveReveal(c.in_drive!).catch((err) => onerror(String(err))) }
                : { label: "Save to Drive", disabled: many || !!c.error || !!job, action: () => void saveClipToDrive(c) },
            ]
          : []),
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
    // Where it can be moved: not into itself or its subfolders, and the
    // deepest subfolder must still fit within MAX_DEPTH.
    const span = Math.max(1, ...org.folders.filter((x) => isWithin(x.id, f.id)).map((x) => x.depth - f.depth + 1));
    const targets: MenuItem[] = [
      { label: "Top level", checked: !f.parent, disabled: !f.parent, action: () => placeFolder(f.id, null, null) },
      ...tree
        .filter((x) => x.id !== f.id && !isWithin(x.id, f.id) && x.depth + span <= MAX_DEPTH)
        .map((x) => ({ label: pathLabel(x.id), dot: x.color, checked: x.id === f.parent, disabled: x.id === f.parent, action: () => placeFolder(f.id, x.id, null) })),
    ];
    const subs = org.folders.filter((x) => isWithin(x.id, f.id)).length;
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "Open", action: () => (folderView = f.id) },
        { label: "Show in File Explorer", action: () => api.folderReveal(f.name).catch((err) => onerror(String(err))) },
        "separator",
        { label: "New subfolder…", disabled: f.depth >= MAX_DEPTH, action: () => newFolder([], f.id) },
        { label: "Rename…", action: () => (dialog = { kind: "rename-folder", folder: f, error: null }) },
        { label: "Move folder to", submenu: targets },
        {
          label: "Color",
          submenu: FOLDER_COLORS.map((c) => ({ label: c.label, dot: c.value, checked: f.color === c.value, action: () => orgDo(api.folderColor(f.name, c.value)) })),
        },
        "separator",
        { label: "Delete folder…", danger: true, action: () => (dialog = { kind: "delete-folder", folder: f, count: counts[f.id] ?? 0, subs }) },
      ],
    };
  }

  // ---- drag and drop

  // Arranging is only possible where the custom order is what's shown. With
  // a search or game filter on, a drop goes next to the visible neighbour;
  // hidden clips keep their places.
  const reorderBlocked = (): string | null => {
    if (!currentFolder) return "Open a folder to arrange clips in your own order";
    if (showingSubs) return "Turn off Include subfolders to arrange clips";
    if (sort !== "custom") return `Sorted by ${sorts.find((x) => x.value === sort)?.label.toLowerCase() ?? sort}. Choose Custom order to arrange`;
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
          thumb: c.error ? null : thumbUrl(c.thumb_key),
        };
      },
      (t: Target, p: Payload) => {
        if (p.kind !== "clips") return;
        const cs = clips.filter((x) => p.ids.includes(x.id));
        if (t.type === "folder") {
          const f = t.id ? (org.folders.find((x) => x.id === t.id) ?? null) : null;
          void moveTo(cs, f);
        } else if (t.type === "insert" && currentFolder) {
          void arrange(p.keys, t.before);
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

  /** Custom order inside the open folder: metadata only, no file moves.
   *  `before` null means "after the last visible clip": with a filter on,
   *  that's right after it, not after the hidden clips at the very end. */
  async function arrange(keys: string[], before: string | null) {
    if (!currentFolder) return;
    const present = inView.map((x) => x.key).filter(Boolean);
    if (before === null && shown.length < inView.length) {
      const pos = new Map(currentFolder.clips.map((k, i) => [k, i]));
      const full = [...present].sort((a, b) => (pos.get(a) ?? 1e9) - (pos.get(b) ?? 1e9));
      const last = shown.filter((c) => !keys.includes(c.key)).at(-1)?.key;
      if (last) before = full.slice(full.indexOf(last) + 1).find((k) => !keys.includes(k)) ?? null;
    }
    await orgDo(api.clipsArrange(currentFolder.name, present, keys, before));
  }

  /** Alt+←/→: move the selection one place in custom order (keyboard drag). */
  async function nudge(dir: -1 | 1) {
    const why = reorderBlocked();
    if (why) return tell(why);
    const keys = shown.filter((c) => selectedSet.has(c.id)).map((c) => c.key);
    if (!keys.length || keys.some((k) => !k)) return;
    const rest = shown.filter((c) => !keys.includes(c.key));
    const firstAt = shown.findIndex((c) => c.key === keys[0]);
    // Index in `rest` the selection currently sits before.
    const at = shown.slice(0, firstAt).filter((c) => !keys.includes(c.key)).length;
    const to = Math.max(0, Math.min(rest.length, at + dir));
    if (to === at) return;
    await arrange(keys, rest[to]?.key ?? null);
    await tick();
    if (selectedId) cardEls[selectedId]?.focus({ preventScroll: true });
    if (selectedId) cardEls[selectedId]?.scrollIntoView({ block: "nearest" });
  }

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
    if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      return nudge(e.key === "ArrowLeft" ? -1 : 1);
    }
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
    { value: "name", label: "Name (A–Z)" },
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
    oncreate={() => newFolder([], null)}
    oncontext={folderMenu}
    onplace={placeFolder}
    ontoggle={() => (collapsed = !collapsed)}
  />

  <div class="main">
  {#each org.pending as p (p.folder)}
    {#if p.ids.length}
      <div class="pending" role="region" aria-label="Folder update">
        <span class="fdot" style:background="var(--folder-{p.color})"></span>
        <span class="ptext">Folders are now real Windows folders. Move the {p.ids.length === 1 ? "clip" : `${p.ids.length} clips`} in <strong>{p.folder}</strong> into one?</span>
        <Button variant="ghost" onclick={() => resolvePending(p, false)}>Leave them</Button>
        <Button variant="primary" onclick={() => resolvePending(p, true)}>Move {p.ids.length === 1 ? "clip" : `${p.ids.length} clips`}</Button>
      </div>
    {/if}
  {/each}
  {#if !view || view.clips.length || loadError}
  <div class="toolbar">
    <div class="chips" role="group" aria-label="Filter by game">
      {#if currentFolder}
        <span class="where" title={pathLabel(currentFolder.id)}>
          <span class="fdot" style:background="var(--folder-{currentFolder.color})"></span>
          {#each currentFolder.id.split("\\").slice(0, -1) as part, k (k)}
            {@const up = currentFolder.id.split("\\").slice(0, k + 1).join("\\")}
            <button type="button" class="crumb" onclick={() => (folderView = up)}>{part}</button><span class="sepc" aria-hidden="true">›</span>
          {/each}
          <span class="leaf">{currentFolder.leaf}</span>
        </span>
        {#if hasSubfolders}
          <button type="button" class="chip" class:on={withSubs} aria-pressed={withSubs} onclick={() => (withSubs = !withSubs)} title="Also show the clips in this folder's subfolders">Include subfolders</button>
        {/if}
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
      <Select label="Sort clips" value={sort} options={sorts} onchange={(v) => setSort(v as Sort)} />
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
      <h2>{currentFolder.leaf} is empty</h2>
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
                folderColor={folderOf.get(c.id)?.color ?? null}
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
        {#if !selected.error}<img src={thumbUrl(selected.thumb_key)} alt="" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.visibility = "hidden")} />{/if}
      </div>
      <div class="sel-meta">
        <strong title={selected.title ?? selected.game}>{selected.title ?? selected.game}</strong>
        <span>{#if selected.title}{selected.game} · {/if}{whenShort(selected.recorded)} · {selected.probe ? clock(selected.probe.duration) : "unreadable"} · {bytes(selected.size)}{#if folderOf.get(selected.id)} · {pathLabel(folderOf.get(selected.id)?.id ?? "")}{/if}</span>
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
{#if job}
  <div class="notice jobbar" class:raised={selectedClips.length > 1 || !!primary} role="status">
    <span>{job.kind === "share" ? "Making a Discord copy of" : "Saving to Google Drive:"} {nameOf(job.clip)} · {Math.round(job.progress * 100)}%</span>
    <button type="button" class="jobcancel" onclick={() => void api.exportCancel()}>Cancel</button>
    <span class="jobprog" style:width="{job.progress * 100}%" aria-hidden="true"></span>
  </div>
{:else if notice}<div class="notice" class:raised={selectedClips.length > 1 || !!primary} role="status">{notice}</div>{/if}
{#if dialog?.kind === "new-folder"}
  {@const d = dialog}
  <Dialog
    title={d.parent ? `New folder in ${org.folders.find((x) => x.id === d.parent)?.leaf ?? d.parent}` : "New folder"}
    confirmLabel={d.moveIds.length ? "Create and move" : "Create"}
    input={{ value: "", placeholder: d.parent ? "e.g. DayZ" : "e.g. DayZ Raids", max: 60 }}
    error={d.error}
    onconfirm={(v) => createFolder(v, d.color, d.moveIds, d.parent)}
    oncancel={() => (dialog = null)}
  >
    {#snippet after()}
      <div class="swatches" role="radiogroup" aria-label="Folder colour">
        {#each FOLDER_COLORS as c (c.value)}
          <button
            type="button"
            class="swatch"
            role="radio"
            aria-checked={d.color === c.value}
            aria-label={c.label}
            title={c.label}
            style:--sw="var(--folder-{c.value})"
            onclick={() => (dialog = { ...d, color: c.value })}
          ></button>
        {/each}
      </div>
    {/snippet}
  </Dialog>
{:else if dialog?.kind === "rename-folder"}
  {@const d = dialog}
  <Dialog title="Rename folder" confirmLabel="Rename" input={{ value: d.folder.leaf, max: 60 }} error={d.error} onconfirm={(v) => renameFolder(d.folder, v)} oncancel={() => (dialog = null)} />
{:else if dialog?.kind === "delete-folder"}
  {@const d = dialog}
  <Dialog title="Delete “{d.folder.leaf}”?" confirmLabel="Delete folder" danger onconfirm={() => deleteFolder(d.folder)} oncancel={() => (dialog = null)}>
    <p>{#if d.subs}Its {d.subs === 1 ? "subfolder goes" : `${d.subs} subfolders go`} too. {/if}{#if d.count}{d.subs ? "All" : "Its"} {d.count === 1 ? "clip is" : `${d.count} clips are`} moved back to {d.count === 1 ? "its game folder" : "their game folders"} first.{:else}It's empty.{/if} No recordings are deleted.</p>
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
  .crumb {
    border: 0;
    padding: 0;
    background: none;
    color: var(--text-muted);
    font: inherit;
    font-weight: var(--fw-regular);
    cursor: pointer;
  }
  .crumb:hover {
    color: var(--text);
    text-decoration: underline;
  }
  .sepc {
    color: var(--text-faint);
    font-weight: var(--fw-regular);
  }
  .where .leaf {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .where {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    max-width: 340px;
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
  .pending {
    display: flex;
    align-items: center;
    gap: var(--s3);
    margin: var(--s3) var(--s5) 0;
    padding: var(--s2) var(--s2) var(--s2) var(--s4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface);
    font-size: var(--fs-sm);
  }
  .pending .fdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .ptext {
    flex: 1;
    min-width: 0;
    color: var(--text-muted);
  }
  .ptext strong {
    color: var(--text);
    font-weight: var(--fw-strong);
  }
  .swatches {
    display: flex;
    gap: var(--s2);
  }
  .swatch {
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--sw);
    cursor: pointer;
  }
  .swatch[aria-checked="true"] {
    box-shadow:
      0 0 0 2px var(--surface-raised),
      0 0 0 4px var(--sw);
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
  /* Above the selection bar when it's showing. */
  .notice.raised {
    bottom: calc(var(--s7) + 48px);
  }
  .jobbar {
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding-right: var(--s2);
    overflow: hidden;
  }
  .jobcancel {
    height: 24px;
    padding: 0 var(--s2);
    border: 0;
    border-radius: var(--radius-pill);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    font-weight: var(--fw-strong);
    cursor: pointer;
  }
  .jobcancel:hover {
    background: var(--surface-hover);
    color: var(--text);
  }
  .jobprog {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    background: var(--accent);
    transition: width var(--t-med) var(--ease);
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
