<script lang="ts">
  import { onMount } from "svelte";
  import { api, clipUrl, type Clip } from "./lib/api";
  import { bytes, clock, codecName, fromWithin, whenShort } from "./lib/format";
  import Button from "./lib/Button.svelte";
  import Timeline from "./lib/Timeline.svelte";

  let {
    clip,
    hasPrev,
    hasNext,
    onclose,
    onstep,
    onerror,
  }: {
    clip: Clip;
    hasPrev: boolean;
    hasNext: boolean;
    onclose: () => void;
    /** -1 previous clip, +1 next clip (in the library's current order) */
    onstep: (dir: -1 | 1) => void;
    onerror: (msg: string) => void;
  } = $props();

  let video = $state<HTMLVideoElement>();
  let time = $state(0);
  let duration = $state(0);
  let buffered = $state(0);
  let paused = $state(true);
  let muted = $state(false);
  let loading = $state(true);
  let failed = $state<string | null>(null);

  // A new clip: reset and autoplay.
  $effect(() => {
    void clip.id;
    time = 0;
    duration = clip.probe?.duration ?? 0;
    buffered = 0;
    loading = true;
    failed = null;
  });

  function toggle() {
    if (!video || failed) return;
    if (video.paused) void video.play().catch(() => {});
    else video.pause();
  }
  function seek(t: number, fast = false) {
    if (!video || failed) return;
    t = Math.min(duration, Math.max(0, t));
    if (fast && typeof video.fastSeek === "function") video.fastSeek(t);
    else video.currentTime = t;
    time = t;
  }
  function frame(dir: number) {
    if (!video) return;
    video.pause();
    seek(time + dir / 60);
  }

  function onkey(e: KeyboardEvent) {
    if (e.ctrlKey || e.metaKey || e.altKey || fromWithin(e, "input, select")) return;
    const handled = (() => {
      switch (e.key) {
        case " ":
        case "k":
        case "K":
          if (fromWithin(e, "button") && e.key === " ") return false;
          toggle();
          return true;
        case "ArrowLeft":
          seek(time - 5);
          return true;
        case "ArrowRight":
          seek(time + 5);
          return true;
        case "j":
        case "J":
          seek(time - 10);
          return true;
        case "l":
        case "L":
          seek(time + 10);
          return true;
        case ",":
          frame(-1);
          return true;
        case ".":
          frame(1);
          return true;
        case "Home":
          seek(0);
          return true;
        case "End":
          seek(duration);
          return true;
        case "m":
        case "M":
          muted = !muted;
          return true;
        case "Escape":
          onclose();
          return true;
        case "PageUp":
          if (hasPrev) onstep(-1);
          return true;
        case "PageDown":
          if (hasNext) onstep(1);
          return true;
      }
      return false;
    })();
    if (handled) e.preventDefault();
  }

  // Stop decoding the moment the window is hidden or loses focus (back to the
  // game): the GPU's video engine is shared with the recorder's encoder.
  function pauseInBackground() {
    if (document.hidden || !document.hasFocus()) video?.pause();
  }

  onMount(() => {
    window.addEventListener("keydown", onkey);
    window.addEventListener("blur", pauseInBackground);
    document.addEventListener("visibilitychange", pauseInBackground);
    return () => {
      window.removeEventListener("keydown", onkey);
      window.removeEventListener("blur", pauseInBackground);
      document.removeEventListener("visibilitychange", pauseInBackground);
      // Release the decoder and the file at once.
      if (video) {
        video.pause();
        video.removeAttribute("src");
        video.load();
      }
    };
  });

  function progress() {
    if (!video) return;
    const b = video.buffered;
    for (let i = 0; i < b.length; i++) {
      if (b.start(i) <= video.currentTime + 0.5 && b.end(i) >= video.currentTime) {
        buffered = b.end(i);
        return;
      }
    }
  }

  const meta = $derived(
    [
      whenShort(clip.recorded),
      clock(clip.probe?.duration ?? 0),
      bytes(clip.size),
      clip.probe ? `${clip.probe.height}p ${codecName(clip.probe.codec)}` : null,
    ]
      .filter(Boolean)
      .join(" · "),
  );

  function mediaError() {
    const code = video?.error?.code;
    const codec = clip.probe ? codecName(clip.probe.codec) : "";
    failed =
      code === 4 && codec && codec !== "H.264"
        ? `This clip is ${codec}, which Windows can't preview without its codec extension.`
        : "This clip couldn't be played here.";
    loading = false;
  }
</script>

<div class="player">
  <div class="bar">
    <Button variant="ghost" onclick={onclose} title="Back to library (Esc)">
      <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M10 3.5 5.5 8l4.5 4.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
      Library
    </Button>
    <div class="title">
      <strong>{clip.game}</strong>
      <span>{meta}</span>
    </div>
    <div class="nav">
      <button type="button" class="icon-btn" aria-label="Previous clip (Page Up)" title="Previous clip (Page Up)" disabled={!hasPrev} onclick={() => onstep(-1)}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M10 3.5 5.5 8l4.5 4.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
      <button type="button" class="icon-btn" aria-label="Next clip (Page Down)" title="Next clip (Page Down)" disabled={!hasNext} onclick={() => onstep(1)}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m6 3.5 4.5 4.5L6 12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
    </div>
    <div class="actions">
      <Button variant="ghost" onclick={() => api.showInFolder(clip.id).catch((e) => onerror(String(e)))}>Show in folder</Button>
      <Button variant="ghost" onclick={() => api.openExternal(clip.id).catch((e) => onerror(String(e)))}>Open in player</Button>
    </div>
  </div>

  <div class="stage">
    {#key clip.id}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={video}
        src={clipUrl(clip.id)}
        autoplay
        preload="auto"
        bind:paused
        bind:muted
        ontimeupdate={() => video && (time = video.currentTime)}
        ondurationchange={() => video && Number.isFinite(video.duration) && (duration = video.duration)}
        onprogress={progress}
        oncanplay={() => (loading = false)}
        onwaiting={() => (loading = true)}
        onplaying={() => (loading = false)}
        onerror={mediaError}
        onclick={toggle}
      ></video>
    {/key}
    {#if failed}
      <div class="overlay">
        <p>{failed}</p>
        <Button variant="primary" onclick={() => api.openExternal(clip.id).catch((e) => onerror(String(e)))}>Open in default player</Button>
      </div>
    {:else if loading}
      <div class="overlay quiet" aria-label="Loading video"><span class="spinner"></span></div>
    {/if}
  </div>

  <div class="controls">
    <button type="button" class="play" aria-label={paused ? "Play (Space)" : "Pause (Space)"} title={paused ? "Play (Space)" : "Pause (Space)"} onclick={toggle} disabled={!!failed}>
      {#if paused}
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5 3v10l8-5z" fill="currentColor" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 3h2.5v10H4.5zM9 3h2.5v10H9z" fill="currentColor" /></svg>
      {/if}
    </button>
    <span class="time"><span class="now">{clock(time)}</span> / {clock(duration)}</span>
    <div class="seek"><Timeline {duration} {time} {buffered} onseek={seek} /></div>
    <button type="button" class="icon-btn" aria-label={muted ? "Unmute (M)" : "Mute (M)"} title={muted ? "Unmute (M)" : "Mute (M)"} onclick={() => (muted = !muted)}>
      {#if muted}
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5L8.5 3v10L5 10H2.5z" fill="currentColor" /><path d="m11 6 3.5 4m0-4L11 10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5L8.5 3v10L5 10H2.5z" fill="currentColor" /><path d="M11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      {/if}
    </button>
  </div>
  <p class="keys">Space play/pause · ←/→ 5 s · J/L 10 s · ,/. frame · Page Up/Down previous/next clip · Esc back</p>
</div>

<style>
  .player {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: var(--s2) var(--s4);
    border-bottom: 1px solid var(--border);
  }
  .title {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
    font-size: var(--fs-sm);
  }
  .title span {
    color: var(--text-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .nav,
  .actions {
    display: flex;
    gap: var(--s1);
    flex: none;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    background: #000;
  }
  video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
    cursor: pointer;
  }
  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s3);
    background: rgb(0 0 0 / 0.6);
    text-align: center;
    padding: var(--s5);
  }
  .overlay.quiet {
    background: transparent;
    pointer-events: none;
  }
  .overlay p {
    margin: 0;
    max-width: 420px;
    color: var(--text);
  }
  .spinner {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 3px solid rgb(255 255 255 / 0.2);
    border-top-color: #fff;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .controls {
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: var(--s3) var(--s4) var(--s1);
    background: var(--surface);
  }
  .play {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    border: 0;
    background: var(--text);
    color: var(--bg);
    cursor: pointer;
    flex: none;
  }
  .play:hover:not(:disabled) {
    background: #fff;
  }
  .play:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .play svg {
    width: 16px;
    height: 16px;
  }
  .time {
    flex: none;
    min-width: 92px;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
  .now {
    color: var(--text);
    font-weight: var(--fw-strong);
  }
  .seek {
    flex: 1;
    min-width: 0;
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: var(--control-h);
    height: var(--control-h);
    border-radius: var(--radius-md);
    border: 0;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    flex: none;
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text);
  }
  .icon-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .icon-btn svg {
    width: 16px;
    height: 16px;
  }
  .keys {
    margin: 0;
    padding: var(--s1) var(--s4) var(--s3);
    background: var(--surface);
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
</style>
