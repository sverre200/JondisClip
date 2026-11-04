<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, isDesktop, type Codec, type InputDevice, type Quality, type Resolution, type Settings, type Snapshot, type TrayView } from "./lib/api";
  import { bytes, estimateBitrate, outputSize, replayLabel, replayShort } from "./lib/format";
  import Banner from "./lib/Banner.svelte";
  import Button from "./lib/Button.svelte";
  import HotkeyField from "./lib/HotkeyField.svelte";
  import Row from "./lib/Row.svelte";
  import Segmented from "./lib/Segmented.svelte";
  import StatusPill from "./lib/StatusPill.svelte";
  import Select from "./lib/Select.svelte";
  import Toggle from "./lib/Toggle.svelte";

  let snap = $state<Snapshot | null>(null);
  let settings = $state<Settings | null>(null);
  let tray = $state<TrayView | null>(null);
  let loadError = $state<string | null>(null);
  let toast = $state<{ text: string; tone: "ok" | "error" } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let startingTray = $state(false);
  let mics = $state<InputDevice[] | null>(null);
  let micsError = $state<string | null>(null);

  async function refreshMics() {
    try {
      mics = await api.listMicrophones();
      micsError = null;
    } catch (e) {
      micsError = String(e);
    }
  }

  // Source resolution for size estimates: the screen in physical pixels,
  // falling back to 1080p if the webview reports nothing usable.
  const screenW = Math.round(window.screen.width * window.devicePixelRatio);
  const screenH = Math.round(window.screen.height * window.devicePixelRatio);
  const srcW = screenW >= 640 ? screenW : 1920;
  const srcH = screenH >= 360 ? screenH : 1080;

  function showToast(text: string, tone: "ok" | "error" = "ok") {
    toast = { text, tone };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), tone === "ok" ? 1600 : 4000);
  }

  async function change(patch: Partial<Settings>) {
    if (!settings) return;
    const before = settings;
    settings = { ...settings, ...patch }; // optimistic
    try {
      settings = await api.update(patch);
      showToast(tray?.running ? "Saved · applied within 2 s" : "Saved");
    } catch (e) {
      settings = before;
      showToast(String(e), "error");
    }
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
      showToast(String(e), "error");
    } finally {
      setTimeout(() => (startingTray = false), 1500);
    }
  }

  async function pickFolder() {
    if (!settings || !snap) return;
    const picked = await api.pickFolder(settings.save_dir || snap.default_save_dir);
    if (picked) await change({ save_dir: picked });
  }

  onMount(() => {
    let poll: ReturnType<typeof setInterval> | undefined;
    (async () => {
      try {
        snap = await api.load();
        settings = snap.settings;
        tray = snap.tray;
        void refreshMics();
      } catch (e) {
        loadError = String(e);
      }
      // Reveal once the content is rendered, so the window never flashes blank.
      // This deliberately doesn't wait for requestAnimationFrame, which a
      // hidden window may never get. The Rust side also has a fallback.
      await tick();
      void api.reveal();
      poll = setInterval(pollTray, 1500);
    })();
    return () => clearInterval(poll);
  });

  const saveDir = $derived(settings && snap ? settings.save_dir || snap.default_save_dir : "");
  const est = $derived.by(() => {
    if (!settings) return null;
    const bps = estimateBitrate(settings, srcW, srcH);
    const out = outputSize(settings, srcW, srcH);
    return {
      perMin: bytes((bps / 8) * 60),
      perClip: bytes((bps / 8) * settings.replay_seconds),
      out: `${out.w}×${out.h}`,
    };
  });
  const micOptions = $derived.by(() => {
    const list = mics ?? [];
    const def = list.find((m) => m.is_default);
    const opts = [{ value: "", label: def ? `Windows default · ${def.name}` : "Windows default" }];
    for (const m of list) opts.push({ value: m.id, label: m.name });
    const chosen = settings?.microphone_device;
    if (chosen && mics && !list.some((m) => m.id === chosen)) {
      opts.push({ value: chosen, label: "Selected microphone (not connected)" });
    }
    return opts;
  });
  const micHint = $derived.by(() => {
    if (micsError) return `Couldn't list microphones: ${micsError}`;
    const chosen = settings?.microphone_device;
    if (chosen && mics && !mics.some((m) => m.id === chosen))
      return "Not connected. The mic track stays silent until it's plugged back in.";
    if (!chosen) return "Follows the Windows default if you change it.";
    return "Always records this microphone, even if the Windows default changes.";
  });
  const hotkeyConflict = $derived(
    !!tray?.running && !!tray.status && !tray.status.hotkey_ok && tray.status.hotkey === settings?.hotkey,
  );

  const resolutions: { value: Resolution; label: string }[] = [
    { value: "source", label: "Native" },
    { value: "1440p", label: "1440p" },
    { value: "1080p", label: "1080p" },
    { value: "720p", label: "720p" },
  ];
  const qualities: { value: Quality; label: string; title: string }[] = [
    { value: "performance", label: "Performance", title: "Smallest files, lightest on the GPU" },
    { value: "balanced", label: "Balanced", title: "Recommended" },
    { value: "high", label: "High", title: "Sharpest motion, biggest files" },
  ];
  const codecs: { value: Codec; label: string; title: string }[] = [
    { value: "auto", label: "H.264", title: "Plays everywhere (recommended)" },
    { value: "hevc", label: "HEVC", title: "Smaller files; needs the HEVC extension or VLC to play" },
    { value: "av1", label: "AV1", title: "Smallest files; newest GPUs only" },
  ];
</script>

<main>
  <header>
    <div class="brand">
      <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="10" fill="none" stroke="var(--accent)" stroke-width="2.5" />
        <circle cx="12" cy="12" r="5" fill="var(--accent)" />
      </svg>
      <div>
        <h1>JondisClip</h1>
        <p class="sub">
          {#if tray?.running && tray.status}{tray.status.summary}{:else if tray}Not recording{:else}&nbsp;{/if}
        </p>
      </div>
    </div>
    <StatusPill {tray} />
  </header>

  {#if tray && !tray.running}
    <Banner>
      JondisClip isn't running, so nothing is being recorded. Your settings still save.
      {#snippet action()}
        <Button variant="primary" onclick={startTray} disabled={startingTray}>
          {startingTray ? "Starting…" : "Start JondisClip"}
        </Button>
      {/snippet}
    </Banner>
  {/if}
  {#if snap?.warnings.length}
    <Banner>Some settings in the file were invalid and were reset: {snap.warnings.join("; ")}</Banner>
  {/if}
  {#if tray?.status?.audio_error}
    <Banner>Audio couldn't start, so clips are recorded without sound. {tray.status.audio_error}</Banner>
  {/if}

  {#if loadError}
    <Banner tone="error">Couldn't load settings: {loadError}</Banner>
  {:else if !settings || !snap}
    <div class="skeleton" aria-busy="true" aria-label="Loading settings">
      {#each Array(6) as _, i (i)}<div class="sk-row"></div>{/each}
    </div>
  {:else}
    <section aria-labelledby="h-replay">
      <h2 id="h-replay">Instant replay</h2>
      <Row label="Replay length" hint="How much of the past the hotkey saves.">
        <Segmented
          label="Replay length"
          value={settings.replay_seconds}
          options={snap.replay_presets.map((s) => ({ value: s, label: replayShort(s), title: replayLabel(s) }))}
          onchange={(v) => change({ replay_seconds: v })}
        />
      </Row>
      <Row label="Save hotkey" hint="Works everywhere, including in fullscreen games.">
        <HotkeyField
          value={settings.hotkey}
          conflict={hotkeyConflict}
          validate={api.checkHotkey}
          onchange={(v) => change({ hotkey: v })}
        />
      </Row>
      <Row label="Save location" hint="Clips go into a folder per game.">
        <span class="path" title={saveDir}>{saveDir}</span>
        <Button onclick={pickFolder} disabled={!isDesktop}>Change…</Button>
        <Button variant="ghost" onclick={() => api.openFolder(saveDir)} title="Open in File Explorer">Open</Button>
        {#snippet below()}
          {#if settings?.save_dir}
            <div class="below-right">
              <Button variant="ghost" onclick={() => change({ save_dir: "" })}>Use default (Videos\JondisClip)</Button>
            </div>
          {/if}
        {/snippet}
      </Row>
      <Row label="Save sound" hint="A short sound confirms the hotkey, even in fullscreen." id="t-sound">
        <Toggle id="t-sound" label="Save sound" checked={settings.save_sound} onchange={(v) => change({ save_sound: v })} />
      </Row>
    </section>

    <section aria-labelledby="h-video">
      <h2 id="h-video">Video</h2>
      <Row label="Quality" hint={est ? `About ${est.perMin} per minute · a ${replayLabel(settings.replay_seconds)} clip is about ${est.perClip}` : undefined}>
        <Segmented label="Quality" value={settings.quality} options={qualities} onchange={(v) => change({ quality: v })} />
      </Row>
      <Row label="Resolution" hint={est ? `Records at ${est.out}. It never upscales.` : undefined}>
        <Segmented label="Resolution" value={settings.resolution} options={resolutions} onchange={(v) => change({ resolution: v })} />
      </Row>
      <Row label="Frame rate" hint="Higher is smoother and uses more space.">
        <Segmented
          label="Frame rate"
          value={settings.fps}
          options={snap.fps_presets.map((f) => ({ value: f, label: `${f}` }))}
          onchange={(v) => change({ fps: v })}
        />
      </Row>
    </section>

    <section aria-labelledby="h-audio">
      <h2 id="h-audio">Audio</h2>
      <Row label="Game and system audio" hint="Everything you hear through your speakers or headset." id="t-sys">
        <Toggle id="t-sys" label="Game and system audio" checked={settings.system_audio} onchange={(v) => change({ system_audio: v })} />
      </Row>
      <Row label="Microphone" hint="Saved as its own track, so it can be removed from a clip later." id="t-mic">
        <Toggle id="t-mic" label="Microphone" checked={settings.microphone} onchange={(v) => change({ microphone: v })} />
      </Row>
      {#if settings.microphone}
        <Row label="Microphone device" hint={micHint} id="s-mic">
          <Select
            id="s-mic"
            label="Microphone device"
            value={settings.microphone_device}
            options={micOptions}
            onopen={refreshMics}
            onchange={(v) => change({ microphone_device: v })}
          />
        </Row>
      {/if}
    </section>

    <section aria-labelledby="h-general">
      <h2 id="h-general">General</h2>
      <Row label="Start with Windows" hint="Start JondisClip in the tray when you sign in." id="t-boot">
        <Toggle id="t-boot" label="Start with Windows" checked={settings.start_with_windows} onchange={(v) => change({ start_with_windows: v })} />
      </Row>
    </section>

    <details>
      <summary>Advanced</summary>
      <Row label="Video codec" hint="H.264 plays everywhere. HEVC and AV1 make smaller files but need extra codecs to play on many PCs.">
        <Segmented
          label="Video codec"
          value={settings.codec === "h264" ? "auto" : settings.codec}
          options={codecs}
          onchange={(v) => change({ codec: v })}
        />
      </Row>
    </details>

    <footer>Changes are saved automatically.</footer>
  {/if}
</main>

{#if toast}
  <div class="toast {toast.tone}" role={toast.tone === "error" ? "alert" : "status"}>
    {#if toast.tone === "ok"}
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
    {/if}
    {toast.text}
  </div>
{/if}

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: var(--s6) var(--s5) var(--s7);
    display: flex;
    flex-direction: column;
    gap: var(--s5);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s4);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: var(--s3);
    min-width: 0;
  }
  .mark {
    width: 32px;
    height: 32px;
    flex: none;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--fs-xl);
    font-weight: var(--fw-strong);
    line-height: 1.2;
  }
  .sub {
    margin: 2px 0 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  h2 {
    margin: 0 0 var(--s1);
    font-size: var(--fs-xs);
    font-weight: var(--fw-strong);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  section {
    display: flex;
    flex-direction: column;
  }
  .path {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl; /* keep the end of long paths visible */
    text-align: left;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .below-right {
    display: flex;
    justify-content: flex-end;
  }
  details {
    border-top: 1px solid var(--border);
    padding-top: var(--s3);
  }
  summary {
    cursor: pointer;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    padding: var(--s1) 0;
    width: max-content;
    border-radius: var(--radius-sm);
  }
  summary:hover {
    color: var(--text);
  }
  details :global(.row:first-of-type) {
    border-top: none;
  }
  footer {
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
  .skeleton {
    display: flex;
    flex-direction: column;
    gap: var(--s3);
  }
  .sk-row {
    height: 44px;
    border-radius: var(--radius-md);
    background: linear-gradient(90deg, var(--surface) 0%, var(--surface-raised) 50%, var(--surface) 100%);
    background-size: 200% 100%;
    animation: shimmer 1.2s linear infinite;
  }
  @keyframes shimmer {
    to {
      background-position: -200% 0;
    }
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: var(--s5);
    transform: translateX(-50%);
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    max-width: calc(100vw - 48px);
    padding: var(--s2) var(--s4);
    border-radius: var(--radius-pill);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-sm);
    animation: rise var(--t-med) var(--ease);
  }
  .toast svg {
    width: 14px;
    height: 14px;
    color: var(--success);
  }
  .toast.error {
    color: var(--text);
    box-shadow: var(--shadow-pop), inset 0 0 0 1px var(--danger);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translate(-50%, 6px);
    }
  }
</style>
