<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, type Clip, type TrayView } from "./lib/api";
  import Banner from "./lib/Banner.svelte";
  import Button from "./lib/Button.svelte";
  import StatusPill from "./lib/StatusPill.svelte";
  import Library from "./Library.svelte";
  import Player from "./Player.svelte";
  import Settings from "./Settings.svelte";

  type Page = "library" | "settings";
  let page = $state<Page | null>(null);
  let tray = $state<TrayView | null>(null);
  let startingTray = $state(false);
  // The clip open in the player, and the list it was opened from (for prev/next).
  let playing = $state<{ clip: Clip; list: Clip[] } | null>(null);
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
      // Reveal once the shell is rendered, so the window never flashes blank.
      // This deliberately doesn't wait for requestAnimationFrame, which a
      // hidden window may never get. The Rust side also has a fallback.
      await tick();
      void api.reveal();
      poll = setInterval(pollTray, 1500);
    })();
    return () => clearInterval(poll);
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
      <nav aria-label="Pages">
        <button type="button" class:on={page === "library"} aria-current={page === "library" ? "page" : undefined} onclick={() => (page = "library")}>Library</button>
        <button type="button" class:on={page === "settings"} aria-current={page === "settings" ? "page" : undefined} onclick={() => (page = "settings")}>Settings</button>
      </nav>
      <div class="right"><StatusPill {tray} /></div>
    </header>

    {#if tray && !tray.running}
      <div class="banner-wrap">
        <Banner>
          JondisClip isn't running, so nothing is being recorded.
          {#snippet action()}
            <Button variant="primary" onclick={startTray} disabled={startingTray}>
              {startingTray ? "Starting…" : "Start JondisClip"}
            </Button>
          {/snippet}
        </Banner>
      </div>
    {/if}
  {/if}

  <div class="content">
    <!-- The library stays mounted behind the player, so filters, selection and
         scroll position survive opening a clip. -->
    {#if page === "library"}
      <div class="fill" hidden={!!playing}>
        <Library hotkey={tray?.status?.hotkey ?? null} onopen={(clip, list) => (playing = { clip, list })} onerror={error} />
      </div>
    {:else if page === "settings"}
      <div class="scroll"><Settings {tray} /></div>
    {/if}
    {#if playing}
      <div class="fill">
        <Player
          clip={playing.clip}
          hasPrev={playing.list.slice(0, playIndex).some((c) => !c.error)}
          hasNext={playing.list.slice(playIndex + 1).some((c) => !c.error)}
          onclose={() => (playing = null)}
          onstep={step}
          onerror={error}
        />
      </div>
    {/if}
  </div>
</div>

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
    justify-content: flex-end;
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
  .scroll {
    height: 100%;
    overflow-y: auto;
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
