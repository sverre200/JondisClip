<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { api, clipUrl, type Clip, type DriveStatus, type ExportEvent, type SharePlan } from "./lib/api";
  import { bytes, clock, clockPrecise, codecName, fromWithin, whenShort } from "./lib/format";
  import { cutPlan, estimateSize } from "./lib/trim";
  import Button from "./lib/Button.svelte";
  import Segmented from "./lib/Segmented.svelte";
  import Timeline from "./lib/Timeline.svelte";

  let {
    clip,
    hasPrev,
    hasNext,
    onclose,
    onstep,
    onerror,
    onsaved,
    onopensaved,
  }: {
    clip: Clip;
    hasPrev: boolean;
    hasNext: boolean;
    onclose: () => void;
    /** -1 previous clip, +1 next clip (in the library's current order) */
    onstep: (dir: -1 | 1) => void;
    onerror: (msg: string) => void;
    /** an edit was saved at `path` (the library should rescan) */
    onsaved: (path: string) => void;
    /** play the saved edit */
    onopensaved: (path: string) => void;
  } = $props();

  // ---- trim state
  let trimStart = $state(0);
  let trimEnd = $state(0);
  let keyframes = $state<number[]>([]);
  /** The user's choice when the start isn't on a keyframe. */
  let preferLossless = $state(false);
  type Job =
    | { state: "idle" }
    | { state: "running"; job: number | null; progress: number; share: boolean; drive?: boolean }
    | { state: "drive"; path: string; size: number; already: boolean }
    | { state: "done"; path: string; size: number; duration: number; lossless: boolean; seconds: number }
    | {
        state: "shared";
        path: string;
        size: number;
        height: number;
        fps: number;
        copied: boolean;
        clipboardError: string | null;
      }
    | { state: "error"; message: string };

  // Save to Drive appears once a Drive folder is set (Settings → Sharing).
  let drive = $state<DriveStatus | null>(null);
  onMount(() => {
    void api
      .driveStatus()
      .then((d) => (drive = d))
      .catch(() => {});
  });

  async function saveToDrive() {
    if (job.state === "running" || !drive?.dir) return;
    job = { state: "running", job: null, progress: 0, share: false, drive: true };
    try {
      const r = await api.driveSave(clip.id, trimStart, trimEnd, !trimmed, !plan.lossless);
      if (r.already || r.job === null) job = { state: "drive", path: r.output, size: clip.size, already: true };
      else if (job.state === "running") job = { ...job, job: r.job };
    } catch (e) {
      job = { state: "error", message: String(e) };
    }
  }

  // What "Share to Discord" would make of the current selection.
  let sharePlan = $state<SharePlan | null>(null);
  $effect(() => {
    const id = clip.id;
    const [s, e] = [trimStart, trimEnd];
    if (!(e > s)) return;
    const t = setTimeout(() => {
      api
        .sharePlan(id, s, e)
        .then((p) => {
          if (clip.id === id) sharePlan = p;
        })
        .catch(() => (sharePlan = null));
    }, 200);
    return () => clearTimeout(t);
  });
  const shareHint = $derived.by(() => {
    if (!sharePlan) return "";
    if (!sharePlan.plan) {
      const m = sharePlan.max_seconds ?? 0;
      return `Too long for ${sharePlan.limit_mb} MB: trim it to under ${clock(m)}`;
    }
    const p = sharePlan.plan;
    return `Discord: ${p.height}p${p.fps} under ${sharePlan.limit_mb} MB${p.rough ? " (will look rough; trim it shorter)" : ""}`;
  });
  let job = $state<Job>({ state: "idle" });

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
    const id = clip.id;
    untrack(() => reset(id));
  });
  function reset(id: string) {
    time = 0;
    duration = clip.probe?.duration ?? 0;
    trimStart = 0;
    trimEnd = duration;
    keyframes = [];
    if (job.state !== "running") job = { state: "idle" };
    buffered = 0;
    loading = true;
    failed = null;
    api
      .clipKeyframes(id)
      .then((k) => {
        if (clip.id === id) keyframes = k;
      })
      .catch(() => {});
  }

  const trimmed = $derived(trimStart > 0.05 || trimEnd < duration - 0.05);
  const plan = $derived(cutPlan(trimStart, trimEnd, keyframes, preferLossless));
  const estimate = $derived(estimateSize(clip.size, duration, plan, trimEnd));

  function setTrim(start: number, end: number, edge: "start" | "end") {
    trimStart = Math.max(0, start);
    trimEnd = Math.min(duration, end);
    if (job.state === "done" || job.state === "shared" || job.state === "drive" || job.state === "error") job = { state: "idle" };
    // Show the frame at the handle being dragged.
    if (video && !video.paused) video.pause();
    previewSeek(edge === "start" ? trimStart : Math.max(trimStart, trimEnd - 1 / 60));
  }

  // While dragging, seeks would queue up (each one reads from disk and
  // decodes) and the picture would trail behind the pointer. Latest wins:
  // during a seek only the newest position is kept, applied when it's done.
  let nextSeek: number | null = null;
  function previewSeek(t: number) {
    if (video?.seeking) {
      nextSeek = t;
      time = t;
      return;
    }
    seek(t, true);
  }
  function seeked() {
    if (nextSeek === null) return;
    const t = nextSeek;
    nextSeek = null;
    seek(t, true);
  }
  function markIn() {
    setTrim(Math.min(time, trimEnd - 0.25), trimEnd, "start");
  }
  function markOut() {
    setTrim(trimStart, Math.max(time, trimStart + 0.25), "end");
  }
  function resetTrim() {
    trimStart = 0;
    trimEnd = duration;
    job = { state: "idle" };
  }

  async function shareToDiscord() {
    if (job.state === "running" || !sharePlan?.plan) return;
    job = { state: "running", job: null, progress: 0, share: true };
    try {
      const r = await api.shareStart(clip.id, trimStart, trimEnd, clip.title ?? null);
      if (job.state === "running") job = { ...job, job: r.job };
    } catch (e) {
      job = { state: "error", message: String(e) };
    }
  }

  async function copyAgain(path: string) {
    try {
      await api.shareCopyAgain(path);
      if (job.state === "shared") job = { ...job, clipboardError: null };
    } catch (e) {
      if (job.state === "shared") job = { ...job, clipboardError: String(e) };
    }
  }

  async function save() {
    if (!trimmed || job.state === "running") return;
    job = { state: "running", job: null, progress: 0, share: false };
    try {
      const r = await api.exportStart(clip.id, trimStart, trimEnd, !plan.lossless);
      if (job.state === "running") job = { ...job, job: r.job };
    } catch (e) {
      job = { state: "error", message: String(e) };
    }
  }
  function cancel() {
    void api.exportCancel();
  }

  function onExport(e: ExportEvent) {
    if (job.state !== "running") return;
    // Only events for this job (a stray "cancelled" from an earlier one is ignored).
    if (job.job !== null && e.job !== job.job) return;
    if (e.kind === "progress") job = { ...job, progress: e.progress };
    else if (e.kind === "done" && e.share) {
      job = {
        state: "shared",
        path: e.path,
        size: e.size,
        height: e.share.height,
        fps: e.share.fps,
        copied: e.share.copied,
        clipboardError: e.clipboard && "Err" in e.clipboard ? e.clipboard.Err : null,
      };
    } else if (e.kind === "done" && job.drive) {
      job = { state: "drive", path: e.path, size: e.size, already: false };
    } else if (e.kind === "done") {
      job = { state: "done", path: e.path, size: e.size, duration: e.duration, lossless: e.video === "copy", seconds: e.seconds };
      onsaved(e.path);
    } else if (e.kind === "error") job = { state: "error", message: e.message };
    else job = { state: "idle" };
  }

  // Stop at the end of the selection (checked every displayed frame). One
  // callback chain per element: re-arming cancels the previous one.
  let frameCb: { v: HTMLVideoElement; id: number } | null = null;
  function stopWatching() {
    if (frameCb) frameCb.v.cancelVideoFrameCallback(frameCb.id);
    frameCb = null;
  }
  function watchEnd() {
    stopWatching();
    const v = video;
    if (!v || typeof v.requestVideoFrameCallback !== "function") return;
    const tick = (_: number, meta: VideoFrameCallbackMetadata) => {
      if (!v.paused && trimmed && meta.mediaTime >= trimEnd - 1 / 120) {
        v.pause();
        v.currentTime = trimEnd;
        time = trimEnd;
      }
      frameCb = { v, id: v.requestVideoFrameCallback(tick) };
    };
    frameCb = { v, id: v.requestVideoFrameCallback(tick) };
  }

  function toggle() {
    if (!video || failed) return;
    if (video.paused) {
      // Play the selection: from its start if we're outside it or at its end.
      if (trimmed && (time < trimStart - 0.01 || time >= trimEnd - 0.05)) seek(trimStart);
      void video.play().catch(() => {});
    } else video.pause();
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
    if ((e.ctrlKey || e.metaKey) && (e.key === "s" || e.key === "S")) {
      e.preventDefault();
      void save();
      return;
    }
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
        case "i":
        case "I":
          markIn();
          return true;
        case "o":
        case "O":
          markOut();
          return true;
        case "Escape":
          if (job.state === "running") return true; // cancel explicitly instead
          if (job.state === "shared" || job.state === "drive") {
            job = { state: "idle" };
            return true;
          }
          onclose();
          return true;
        case "PageUp":
          if (hasPrev && job.state !== "running") onstep(-1);
          return true;
        case "PageDown":
          if (hasNext && job.state !== "running") onstep(1);
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
    let unlisten: (() => void) | undefined;
    let gone = false;
    void api.onExport(onExport).then((u) => {
      if (gone) u();
      else unlisten = u;
    });
    window.addEventListener("keydown", onkey);
    window.addEventListener("blur", pauseInBackground);
    document.addEventListener("visibilitychange", pauseInBackground);
    return () => {
      gone = true;
      unlisten?.();
      stopWatching();
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
      clip.title ? clip.game : null,
      whenShort(clip.recorded),
      clock(clip.probe?.duration ?? 0),
      bytes(clip.size),
      clip.probe ? `${clip.probe.height}p ${codecName(clip.probe.codec)}` : null,
    ]
      .filter(Boolean)
      .join(" · "),
  );

  // The element's duration can differ slightly from the header's: keep an
  // untouched end handle at the real end.
  function durationChanged() {
    if (!video || !Number.isFinite(video.duration)) return;
    const was = duration;
    duration = video.duration;
    if (trimEnd >= was - 0.05 || trimEnd > duration) trimEnd = duration;
  }

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
    <Button variant="ghost" onclick={onclose} disabled={job.state === "running"} title={job.state === "running" ? "Saving… cancel or wait first" : "Back to library (Esc)"}>
      <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M10 3.5 5.5 8l4.5 4.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
      Library
    </Button>
    <div class="title">
      <strong title={clip.title ?? clip.game}>{clip.title ?? clip.game}</strong>
      <span>{meta}</span>
    </div>
    <div class="nav">
      <button type="button" class="icon-btn" aria-label="Previous clip (Page Up)" title="Previous clip (Page Up)" disabled={!hasPrev || job.state === "running"} onclick={() => onstep(-1)}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M10 3.5 5.5 8l4.5 4.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
      <button type="button" class="icon-btn" aria-label="Next clip (Page Down)" title="Next clip (Page Down)" disabled={!hasNext || job.state === "running"} onclick={() => onstep(1)}>
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
        ontimeupdate={() => video && nextSeek === null && (time = video.currentTime)}
        onseeked={seeked}
        ondurationchange={durationChanged}
        onprogress={progress}
        oncanplay={() => (loading = false)}
        onloadedmetadata={watchEnd}
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
    <div class="seek">
      <Timeline {duration} {time} {buffered} trim={{ start: trimStart, end: trimEnd }} onseek={seek} ontrim={setTrim} />
    </div>
    <button type="button" class="icon-btn" aria-label={muted ? "Unmute (M)" : "Mute (M)"} title={muted ? "Unmute (M)" : "Mute (M)"} onclick={() => (muted = !muted)}>
      {#if muted}
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5L8.5 3v10L5 10H2.5z" fill="currentColor" /><path d="m11 6 3.5 4m0-4L11 10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5L8.5 3v10L5 10H2.5z" fill="currentColor" /><path d="M11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      {/if}
    </button>
  </div>
  <div class="edit" aria-label="Trim and save">
    <div class="sel-info">
      {#if trimmed}
        <strong>{clockPrecise(trimStart)} – {clockPrecise(trimEnd)}</strong>
        <span>{clockPrecise(trimEnd - plan.start)} · ≈ {bytes(estimate)}{shareHint ? ` · ${shareHint}` : ""}</span>
      {:else}
        <strong>Trim</strong>
        <span>Drag the red handles, or press I and O at the playhead{shareHint ? ` · ${shareHint}` : ""}</span>
      {/if}
    </div>

    <div class="mode">
      {#if job.state !== "idle" && job.state !== "error"}
        <!-- the cut mode only matters before saving -->
      {:else if trimmed && plan.onKeyframe}
        <span class="badge ok" title="The start is on a keyframe, so the clip is copied exactly, with no quality loss">Lossless · exact</span>
      {:else if trimmed}
        <Segmented
          label="How to cut"
          value={preferLossless ? "lossless" : "exact"}
          options={[
            { value: "exact", label: "Exact", title: "Starts on the frame you chose. Video is re-encoded at high quality; audio is copied." },
            { value: "lossless", label: `Lossless · ${plan.snapBy.toFixed(1)} s earlier`, title: "No quality loss, but the clip starts at the previous keyframe" },
          ]}
          onchange={(v) => (preferLossless = v === "lossless")}
        />
      {/if}
    </div>

    <div class="save">
      {#if job.state === "running"}
        <span class="what">{job.share ? "Preparing for Discord…" : job.drive ? "Copying to Drive folder…" : "Saving…"}</span>
        <div class="progress" role="progressbar" aria-label={job.share ? "Preparing for Discord" : "Saving clip"} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(job.progress * 100)}>
          <div style:width="{Math.round(job.progress * 100)}%"></div>
        </div>
        <span class="pct">{Math.round(job.progress * 100)}%</span>
        <Button variant="ghost" onclick={cancel}>Cancel</Button>
      {:else if job.state === "done"}
        {@const saved = job}
        <span class="done" role="status">
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
          Saved {clockPrecise(saved.duration)} · {bytes(saved.size)}{saved.lossless ? " · lossless" : ""}
        </span>
        <Button variant="ghost" onclick={() => onopensaved(saved.path)}>Play it</Button>
        <Button variant="ghost" onclick={resetTrim}>New trim</Button>
      {:else if job.state === "drive"}
        {@const dv = job}
        <span class="done" role="status" title={dv.path}>
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
          {dv.already ? "Already in your Google Drive folder" : "Saved in your Google Drive folder"}<span class="sub">· uploads when Google Drive for desktop syncs</span>
        </span>
        <Button variant="ghost" onclick={() => api.driveReveal(dv.path).catch((e) => onerror(String(e)))}>Show file</Button>
        <Button variant="ghost" onclick={() => (job = { state: "idle" })}>Done</Button>
      {:else if job.state === "shared"}
        {@const sh = job}
        {#if sh.clipboardError}
          <span class="err" role="alert" title={sh.clipboardError}>Ready, but not on the clipboard: {sh.clipboardError}</span>
        {:else}
          <span class="done" role="status" title="{sh.height}p{sh.fps} · {bytes(sh.size)}{sh.copied ? ' · no re-encode' : ''}">
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
            Copied: paste it in Discord with Ctrl+V · {sh.height}p{sh.fps} · {bytes(sh.size)}
          </span>
        {/if}
        <Button variant="ghost" onclick={() => copyAgain(sh.path)}>Copy again</Button>
        <Button variant="ghost" onclick={() => api.shareReveal(sh.path).catch((e) => onerror(String(e)))}>Show file</Button>
        <Button variant="ghost" onclick={() => (job = { state: "idle" })}>Done</Button>
      {:else}
        {#if job.state === "error"}<span class="err" role="alert" title={job.message}>{job.message}</span>{/if}
        {#if trimmed}<Button variant="ghost" onclick={resetTrim}>Reset</Button>{/if}
        <Button onclick={save} disabled={!trimmed || !!failed} title="Save the selection as a new clip next to the original (Ctrl+S). The original is kept.">
          Save clip
        </Button>
        {#if drive?.dir}
          <Button
            onclick={saveToDrive}
            disabled={!!failed}
            title={trimmed
              ? "Save the selection into your Google Drive folder. Google Drive for desktop uploads it from there."
              : "Copy this clip into your Google Drive folder (checked after copying). Google Drive for desktop uploads it from there."}
          >
            Save to Drive
          </Button>
        {/if}
        <Button
          variant="primary"
          onclick={shareToDiscord}
          disabled={!!failed || !sharePlan?.plan}
          title={sharePlan?.plan ? `Makes a copy under ${sharePlan.limit_mb} MB and puts the file on the clipboard. Paste it in any Discord chat.` : shareHint}
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M8 2v8M4.5 5.5 8 2l3.5 3.5M3 10v3h10v-3" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
          Share to Discord
        </Button>
      {/if}
    </div>
  </div>
  <p class="keys">Space play/pause · I/O set start/end · middle-drag moves the selection · Ctrl+S save · ←/→ 5 s · J/L 10 s · ,/. frame · Page Up/Down other clips · Esc back</p>
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
  .edit {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    gap: var(--s4);
    padding: var(--s2) var(--s4) 0;
    background: var(--surface);
    min-height: 48px;
  }
  .sel-info {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
  .sel-info span {
    color: var(--text-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mode {
    display: flex;
    justify-content: center;
  }
  .badge {
    padding: 4px 10px;
    border-radius: var(--radius-pill);
    font-size: var(--fs-xs);
    font-weight: var(--fw-strong);
  }
  .badge.ok {
    background: var(--success-soft);
    color: var(--success);
  }
  .save {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s2);
    min-width: 0;
  }
  .progress {
    width: 160px;
    height: 6px;
    border-radius: var(--radius-pill);
    background: var(--surface-press);
    overflow: hidden;
  }
  .progress div {
    height: 100%;
    background: var(--accent);
    transition: width var(--t-fast) linear;
  }
  .what {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .pct {
    width: 40px;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
  .done {
    display: inline-flex;
    align-items: center;
    gap: var(--s1);
    color: var(--success);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .done .sub {
    color: var(--text-muted);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .keys {
    margin: 0;
    padding: var(--s1) var(--s4) var(--s3);
    background: var(--surface);
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
</style>
