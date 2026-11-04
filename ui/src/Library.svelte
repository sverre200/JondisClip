<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { api, thumbUrl, type Clip, type LibraryView } from "./lib/api";
  import { bytes, clock, dayLabel, fromWithin, whenShort } from "./lib/format";
  import Banner from "./lib/Banner.svelte";
  import Button from "./lib/Button.svelte";
  import ClipCard from "./lib/ClipCard.svelte";
  import Select from "./lib/Select.svelte";

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

  type Sort = "newest" | "oldest" | "longest" | "largest";
  let view = $state<LibraryView | null>(null);
  let loadError = $state<string | null>(null);
  let refreshing = $state(false);
  let game = $state(""); // "" = all games
  let query = $state("");
  let sort = $state<Sort>("newest");
  let selectedId = $state<string | null>(null);
  const cardEls: Record<string, HTMLButtonElement | undefined> = $state({});
  let gridEl = $state<HTMLDivElement>();

  // One scan at a time; a request during a scan runs once after it.
  let again = false;
  async function refresh() {
    if (refreshing) {
      again = true;
      return;
    }
    refreshing = true;
    try {
      view = await api.libraryScan();
      loadError = null;
      onscanned?.(view.clips);
      if (selectedId && !view.clips.some((c) => c.id === selectedId)) selectedId = null;
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

  const games = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const c of view?.clips ?? []) counts.set(c.game, (counts.get(c.game) ?? 0) + 1);
    return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    let list = (view?.clips ?? []).filter(
      (c) => (!game || c.game === game) && (!q || c.game.toLowerCase().includes(q) || c.file_name.toLowerCase().includes(q)),
    );
    const dur = (c: Clip) => c.probe?.duration ?? -1;
    if (sort === "oldest") list = [...list].reverse();
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

  const selected = $derived(shown.find((c) => c.id === selectedId) ?? null);
  // Roving tab stop: the selected card, else the first.
  const tabStop = $derived(selected?.id ?? shown[0]?.id ?? null);

  function select(c: Clip) {
    selectedId = c.id;
  }

  function open(c: Clip) {
    if (c.error) {
      onerror(`This clip can't be read: ${c.error}`);
      return;
    }
    onopen(c, shown);
  }

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
    if (!shown.length || fromWithin(e, "input, select")) return;
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
        if (selected) {
          e.preventDefault();
          open(selected);
        }
        return;
      case "Escape":
        selectedId = null;
        return;
      default:
        return;
    }
    e.preventDefault();
    next = Math.min(shown.length - 1, Math.max(0, next));
    const target = shown[next];
    if (!target) return;
    selectedId = target.id;
    await tick();
    const el = cardEls[target.id];
    el?.focus({ preventScroll: true });
    el?.scrollIntoView({ block: "nearest" });
  }

  const sorts = [
    { value: "newest", label: "Newest first" },
    { value: "oldest", label: "Oldest first" },
    { value: "longest", label: "Longest first" },
    { value: "largest", label: "Largest first" },
  ];
  const total = $derived(shown.reduce((n, c) => n + c.size, 0));
</script>

<div class="lib">
  {#if !view || view.clips.length || loadError}
  <div class="toolbar">
    <div class="chips" role="group" aria-label="Filter by game">
      <button type="button" class="chip" class:on={!game} aria-pressed={!game} onclick={() => (game = "")}>
        All games <span class="n">{view?.clips.length ?? 0}</span>
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
  {:else if !shown.length}
    <div class="empty">
      <h2>No clips match</h2>
      <p>Nothing matches “{query}”{game ? ` in ${game}` : ""}.</p>
      <Button onclick={() => ((query = ""), (game = ""))}>Clear filters</Button>
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="scroll pad" bind:this={gridEl} onkeydown={onkey}>
      {#each groups as g (g.label)}
        <section class="group" aria-label={g.label ?? "Clips"}>
          {#if g.label}
            <h2>{g.label} <span>{g.clips.length} {g.clips.length === 1 ? "clip" : "clips"}</span></h2>
          {/if}
          <div class="grid">
            {#each g.clips as c (c.id)}
              <ClipCard
                clip={c}
                selected={c.id === selectedId}
                tabindex={c.id === tabStop ? 0 : -1}
                onselect={() => select(c)}
                onopen={() => open(c)}
                bind:el={cardEls[c.id]}
              />
            {/each}
          </div>
        </section>
      {/each}
      <p class="foot">{shown.length} {shown.length === 1 ? "clip" : "clips"} · {bytes(total)} · <button type="button" class="link" onclick={() => api.openFolder(view?.root ?? "")}>{view.root}</button></p>
    </div>
  {/if}

  {#if selected}
    <div class="selbar" role="region" aria-label="Selected clip">
      <div class="sel-thumb">
        {#if !selected.error}<img src={thumbUrl(selected.id)} alt="" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.visibility = "hidden")} />{/if}
      </div>
      <div class="sel-meta">
        <strong>{selected.game}</strong>
        <span>{whenShort(selected.recorded)} · {selected.probe ? clock(selected.probe.duration) : "unreadable"} · {bytes(selected.size)}</span>
      </div>
      <div class="sel-actions">
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

<style>
  .lib {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
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
