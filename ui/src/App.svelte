<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, type Clip, type TrayView } from "./lib/api";
  import Banner from "./lib/Banner.svelte";
  import Button from "./lib/Button.svelte";
  import StatusPill from "./lib/StatusPill.svelte";
  import Library from "./Library.svelte";
  import Player from "./Player.svelte";
  import Settings from "./Settings.svelte";
  import Welcome from "./Welcome.svelte";
  import { t } from "./lib/i18n.svelte";

  type Page = "library" | "settings";
  let page = $state<Page | null>(null);
  let tray = $state<TrayView | null>(null);
  let startingTray = $state(false);
  // First run: the welcome screen covers the app until it's completed.
  let welcome = $state(false);
  // The clip open in the player, and the list it was opened from (for prev/next).
  let playing = $state<{ clip: Clip; list: Clip[] } | null>(null);
  // Latest scan, so a saved edit can be opened as soon as the library sees it.
  let clips = $state<Clip[]>([]);
  let refreshToken = $state(0);
  let openWhenScanned: string | null = null;
  const samePath = (a: string, b: string) => a.replaceAll("/", "\\").toLowerCase() === b.replaceAll("/", "\\").toLowerCase();

  // A clip renamed in the player: after the rescan, the player follows the
  // file (its id changed with its name) and keeps its place in the list.
  let renamed: { id: string; key: string; path: string | null; done: () => void } | null = null;
  const fileName = (p: string) => p.replaceAll("/", "\\").split("\\").pop()!.toLowerCase();
  async function renamePlaying(title: string) {
    if (!playing) return;
    const c = playing.clip;
    let path: string | null = null;
    try {
      path = await api.clipTitle(c.id, c.key, title);
    } catch (e) {
      error(String(e)); // the title may still be saved: rescan either way
    }
    await new Promise<void>((done) => {
      renamed = { id: c.id, key: c.key, path, done };
      refreshToken++;
      setTimeout(done, 4000); // never leave the player waiting on a failed scan
    });
  }

  function scanned(list: Clip[]) {
    clips = list;
    if (renamed) {
      const r = renamed;
      renamed = null;
      // By path; or the same recording under the new file name (should the
      // two spell the folder differently); or unchanged, if the rename failed.
      const now =
        (r.path && list.find((x) => samePath(x.path, r.path!))) ||
        (r.path && list.find((x) => x.key === r.key && fileName(x.path) === fileName(r.path!))) ||
        list.find((x) => x.id === r.id);
      if (now && playing?.clip.id === r.id) playing = { clip: now, list: playing.list.map((x) => (x.id === r.id ? now : x)) };
      r.done();
    }
    if (openWhenScanned) {
      const c = list.find((x) => samePath(x.path, openWhenScanned!));
      if (c) playing = { clip: c, list };
      openWhenScanned = null;
    }
  }
  function openSaved(path: string) {
    const c = clips.find((x) => samePath(x.path, path));
    if (c) playing = { clip: c, list: clips };
    else {
      openWhenScanned = path;
      refreshToken++;
    }
  }

  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  function error(msg: string) {
    toast = msg;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 4000);
  }

  async function pollTray() {
    try {
      tray = await api.trayStatus();
    } catch {
      /* keep the last known state */
    }
  }

  // The button reacts on the click; the tray's status takes over when it arrives.
  let installing = $state(false);
  async function installUpdate() {
    installing = true;
    try {
      await api.installUpdate();
      for (let i = 0; i < 12 && !tray?.status?.update_state; i++) {
        await new Promise((r) => setTimeout(r, 250));
        await pollTray();
      }
    } catch (e) {
      error(String(e));
    } finally {
      installing = false;
    }
  }
  // Follow a download closely, so the percentage moves.
  $effect(() => {
    if (!tray?.status?.update_state) return;
    const timer = setInterval(pollTray, 500);
    return () => clearInterval(timer);
  });

  async function startTray() {
    startingTray = true;
    try {
      await api.startTray();
      setTimeout(pollTray, 1500);
    } catch (e) {
      error(String(e));
    } finally {
      setTimeout(() => (startingTray = false), 1500);
    }
  }

  onMount(() => {
    let poll: ReturnType<typeof setInterval> | undefined;
    (async () => {
      page = await api.initialPage().catch(() => "library" as const);
      void pollTray();
      void api
        .load()
        .then((s) => (welcome = !s.settings.setup_done))
        .catch(() => {});
      // Reveal once the shell is rendered, so the window never flashes blank.
      // This deliberately doesn't wait for requestAnimationFrame, which a
      // hidden window may never get. The Rust side also has a fallback.
      await tick();
      void api.reveal();
      poll = setInterval(pollTray, 1500);
    })();
    // The tray's "Settings…" (or a left-click) on the open window: switch to
    // the page it asked for. The player stays open; the page shows behind it.
    const onFocus = async () => {
      const want = await api.takePageRequest().catch(() => null);
      if (want && page) page = want;
    };
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(poll);
      window.removeEventListener("focus", onFocus);
    };
  });

  function step(dir: -1 | 1) {
    if (!playing) return;
    const i = playing.list.findIndex((c) => c.id === playing!.clip.id);
    let j = i + dir;
    while (playing.list[j]?.error) j += dir; // skip unreadable clips
    const next = playing.list[j];
    if (next) playing = { clip: next, list: playing.list };
  }

  const playIndex = $derived(playing ? playing.list.findIndex((c) => c.id === playing!.clip.id) : -1);
</script>

<div class="shell">
  {#if !playing}
    <header>
      <div class="brand">
        <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="12" cy="12" r="10" fill="none" stroke="var(--accent)" stroke-width="2.5" />
          <circle cx="12" cy="12" r="5" fill="var(--accent)" />
        </svg>
        <span class="word">JondisClip</span>
      </div>
      <nav aria-label={t("Pages")}>
        <button type="button" class:on={page === "library"} aria-current={page === "library" ? "page" : undefined} onclick={() => (page = "library")}>{t("Library")}</button>
        <button type="button" class:on={page === "settings"} aria-current={page === "settings" ? "page" : undefined} onclick={() => (page = "settings")}>{t("Settings")}</button>
      </nav>
      <div class="right">
        {#if tray?.running && tray.status?.update_version && !tray.status.dev_build}
          <!-- Always in view on both pages: the tray menu and Settings have the same offer. -->
          <Button
            variant="primary"
            onclick={installUpdate}
            disabled={!!tray.status.update_state || installing}
            title={t("Downloads the new version and restarts JondisClip. Recording continues while it downloads.")}
          >
            {#if tray.status.update_state || installing}
              <span class="spin" aria-hidden="true"></span>
              {tray.status.update_state === "installing" ? t("Installing…") : tray.status.update_progress != null ? t("Downloading… {percent} %", { percent: tray.status.update_progress }) : t("Downloading…")}
            {:else}
              {t("Update to {version}", { version: tray.status.update_version })}
            {/if}
          </Button>
        {/if}
        <StatusPill {tray} />
      </div>
    </header>

    {#if tray && !tray.running}
      <div class="banner-wrap">
        <Banner>
          {t("JondisClip isn't running, so nothing is being recorded.")}
          {#snippet action()}
            <Button variant="primary" onclick={startTray} disabled={startingTray}>
              {startingTray ? t("Starting…") : t("Start JondisClip")}
            </Button>
          {/snippet}
        </Banner>
      </div>
    {/if}
  {/if}

  <div class="content">
    <!-- The library stays mounted behind the player, so filters, selection and
         scroll position survive opening a clip. -->
    <!-- The library stays mounted while Settings is shown too, so switching
         pages never rebuilds the grid or rescans (no flash or jump). -->
    {#if page}
      <div class="fill" hidden={!!playing || page !== "library"}>
        <Library
          hotkey={tray?.status?.hotkey ?? null}
          {refreshToken}
          onscanned={scanned}
          onopen={(clip, list) => (playing = { clip, list })}
          onerror={error}
        />
      </div>
    {/if}
    {#if page === "settings"}
      <div class="fill"><Settings {tray} onpoll={pollTray} /></div>
    {/if}
    {#if playing}
      <div class="fill">
        <Player
          clip={playing.clip}
          hasPrev={playing.list.slice(0, playIndex).some((c) => !c.error)}
          hasNext={playing.list.slice(playIndex + 1).some((c) => !c.error)}
          onclose={() => (playing = null)}
          onstep={step}
          onrename={renamePlaying}
          onerror={error}
          onsaved={() => refreshToken++}
          onopensaved={openSaved}
        />
      </div>
    {/if}
  </div>
</div>

{#if welcome}
  <Welcome {tray} ondone={() => (welcome = false)} />
{/if}

{#if toast}
  <div class="toast" role="alert">{toast}</div>
{/if}

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  header {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: var(--s4);
    height: 52px;
    padding: 0 var(--s5);
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: var(--s2);
  }
  .mark {
    width: 22px;
    height: 22px;
  }
  .word {
    font-family: var(--font-display);
    font-size: var(--fs-md);
    font-weight: var(--fw-strong);
  }
  nav {
    display: flex;
    gap: 2px;
    padding: 3px;
    border-radius: var(--radius-md);
    background: var(--surface);
    border: 1px solid var(--border);
  }
  nav button {
    height: 28px;
    padding: 0 var(--s4);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    cursor: pointer;
  }
  nav button:hover {
    color: var(--text);
  }
  nav button.on {
    background: var(--surface-hover);
    color: var(--text);
  }
  .right {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s3);
  }
  .spin {
    display: inline-block;
    width: 12px;
    height: 12px;
    margin-right: var(--s2);
    vertical-align: -1px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .banner-wrap {
    padding: var(--s3) var(--s5) 0;
  }
  .content {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .fill {
    position: absolute;
    inset: 0;
  }
  .fill[hidden] {
    display: none;
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: var(--s7);
    transform: translateX(-50%);
    max-width: calc(100vw - 48px);
    padding: var(--s2) var(--s4);
    border-radius: var(--radius-pill);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop), inset 0 0 0 1px var(--danger);
    font-size: var(--fs-sm);
    z-index: 10;
  }
</style>
