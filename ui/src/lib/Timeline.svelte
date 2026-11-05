<script lang="ts">
  import { clock, clockPrecise } from "./format";

  // Seek bar with an optional trim selection: the area between the handles is
  // what gets saved; outside it is dimmed. Handles drag; the playhead drags.
  let {
    duration,
    time,
    buffered = 0,
    trim = null,
    onseek,
    ontrim,
  }: {
    duration: number;
    time: number;
    /** seconds buffered ahead (for the faint fill) */
    buffered?: number;
    /** selection in seconds, or null for a plain seek bar */
    trim?: { start: number; end: number } | null;
    /** `fast` while dragging: the player may seek to the nearest keyframe */
    onseek: (t: number, fast?: boolean) => void;
    /** new selection; `edge` says which handle moved (to preview that frame) */
    ontrim?: (start: number, end: number, edge: "start" | "end") => void;
  } = $props();

  let track = $state<HTMLDivElement>();
  let hover = $state<number | null>(null);
  let drag = $state<"head" | "start" | "end" | "range" | null>(null);
  // Middle-button drag moves the whole selection: where it was grabbed.
  let grab = { at: 0, start: 0, end: 0 };

  /** Shortest selection: a quarter of a second. */
  const MIN = 0.25;
  const pct = (t: number) => (duration > 0 ? Math.min(100, Math.max(0, (t / duration) * 100)) : 0);

  function at(e: PointerEvent): number {
    if (!track) return 0;
    const r = track.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) * duration;
  }

  // While dragging, at most one seek per frame: each seek makes the video
  // element read from disk, and pointer events can come at 240 Hz.
  let pending: number | null = null;
  let frame = 0;
  function seekSoon(t: number) {
    pending = t;
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      if (pending !== null) onseek(pending, true);
      pending = null;
    });
  }

  function setEdge(edge: "start" | "end", t: number) {
    if (!trim || !ontrim) return;
    if (edge === "start") ontrim(Math.min(t, trim.end - MIN), trim.end, edge);
    else ontrim(trim.start, Math.max(t, trim.start + MIN), edge);
  }

  function down(e: PointerEvent, what: "head" | "start" | "end") {
    // Middle button anywhere on the bar: slide the selection, length kept.
    if (e.button === 1 && trim && ontrim) {
      e.preventDefault(); // no autoscroll
      e.stopPropagation();
      drag = "range";
      grab = { at: at(e), start: trim.start, end: trim.end };
      try {
        track?.setPointerCapture(e.pointerId);
      } catch {
        /* see below */
      }
      return;
    }
    if (e.button !== 0) return;
    e.stopPropagation();
    drag = what;
    try {
      track?.setPointerCapture(e.pointerId);
    } catch {
      /* synthetic or already-released pointer: dragging still works inside the bar */
    }
    const t = at(e);
    if (what === "head") seekSoon(t);
    else setEdge(what, t);
  }
  function move(e: PointerEvent) {
    const t = at(e);
    hover = t;
    if (drag === "head") seekSoon(t);
    else if (drag === "range") slide(t);
    else if (drag) setEdge(drag, t);
  }

  function slide(t: number) {
    if (!ontrim) return;
    const len = grab.end - grab.start;
    const start = Math.min(duration - len, Math.max(0, grab.start + t - grab.at));
    ontrim(start, start + len, "start");
  }
  function up(e: PointerEvent) {
    if (!drag) return;
    const was = drag;
    drag = null;
    cancelAnimationFrame(frame);
    frame = 0;
    pending = null;
    if (was === "head") onseek(at(e)); // exact position on release
  }

  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 1 / 60 : 5;
    let t: number | null = null;
    if (e.key === "ArrowRight") t = time + step;
    else if (e.key === "ArrowLeft") t = time - step;
    else if (e.key === "Home") t = trim ? trim.start : 0;
    else if (e.key === "End") t = trim ? trim.end : duration;
    if (t === null) return;
    e.preventDefault();
    e.stopPropagation();
    onseek(Math.min(duration, Math.max(0, t)));
  }

  function handleKey(e: KeyboardEvent, edge: "start" | "end") {
    if (!trim) return;
    const step = e.shiftKey ? 1 / 60 : 1;
    const cur = edge === "start" ? trim.start : trim.end;
    let t: number | null = null;
    // Alt+arrows on either handle move the whole selection (keyboard
    // equivalent of the middle-button drag).
    if (e.altKey && (e.key === "ArrowRight" || e.key === "ArrowLeft") && ontrim) {
      e.preventDefault();
      e.stopPropagation();
      const len = trim.end - trim.start;
      const start = Math.min(duration - len, Math.max(0, trim.start + (e.key === "ArrowRight" ? step : -step)));
      ontrim(start, start + len, "start");
      return;
    }
    if (e.key === "ArrowRight") t = cur + step;
    else if (e.key === "ArrowLeft") t = cur - step;
    if (t === null) return;
    e.preventDefault();
    e.stopPropagation();
    setEdge(edge, Math.min(duration, Math.max(0, t)));
  }

  const tip = $derived(
    (drag === "start" || drag === "range") && trim ? trim.start : drag === "end" && trim ? trim.end : hover,
  );
</script>

<div
  class="tl"
  class:dragging={drag !== null}
  class:range={drag === "range"}
  class:trimming={!!trim}
  bind:this={track}
  onpointerdown={(e) => down(e, "head")}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  onpointerleave={() => (hover = null)}
  onmousedown={(e) => e.button === 1 && trim && e.preventDefault()}
  onauxclick={(e) => e.button === 1 && e.preventDefault()}
  role="presentation"
>
  <div class="rail">
    <div class="buf" style:width="{pct(buffered)}%"></div>
    {#if trim}
      <div class="sel" style:left="{pct(trim.start)}%" style:width="{pct(trim.end) - pct(trim.start)}%"></div>
    {:else}
      <div class="fill" style:width="{pct(time)}%"></div>
    {/if}
    {#if hover !== null && drag === null}<div class="ghost" style:left="{pct(hover)}%"></div>{/if}
  </div>
  {#if trim}
    <div class="shade" style:left="0" style:width="{pct(trim.start)}%"></div>
    <div class="shade" style:left="{pct(trim.end)}%" style:right="0"></div>
    <div
      class="handle start"
      style:left="{pct(trim.start)}%"
      role="slider"
      tabindex="0"
      aria-label="Start of the selection"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(trim.start)}
      aria-valuetext={clockPrecise(trim.start)}
      onpointerdown={(e) => down(e, "start")}
      onkeydown={(e) => handleKey(e, "start")}
    ></div>
    <div
      class="handle end"
      style:left="{pct(trim.end)}%"
      role="slider"
      tabindex="0"
      aria-label="End of the selection"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(trim.end)}
      aria-valuetext={clockPrecise(trim.end)}
      onpointerdown={(e) => down(e, "end")}
      onkeydown={(e) => handleKey(e, "end")}
    ></div>
  {/if}
  <div
    class="head"
    style:left="{pct(time)}%"
    role="slider"
    tabindex="0"
    aria-label="Position"
    aria-valuemin={0}
    aria-valuemax={Math.round(duration)}
    aria-valuenow={Math.round(time)}
    aria-valuetext="{clock(time)} of {clock(duration)}"
    onkeydown={key}
  ></div>
  {#if tip !== null}
    <div class="tip" style:left="{pct(tip)}%">{drag === "start" || drag === "end" || drag === "range" ? clockPrecise(tip) : clock(tip)}</div>
  {/if}
</div>

<style>
  .tl {
    position: relative;
    height: 28px;
    cursor: pointer;
    touch-action: none;
  }
  .tl.trimming {
    height: 40px;
  }
  .tl.range {
    cursor: grabbing;
  }
  .rail {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 6px;
    margin-top: -3px;
    border-radius: var(--radius-pill);
    background: var(--surface-press);
    overflow: hidden;
  }
  .trimming .rail {
    height: 28px;
    margin-top: -14px;
    border-radius: var(--radius-sm);
  }
  .buf {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--border-strong);
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
  }
  .sel {
    position: absolute;
    top: 0;
    bottom: 0;
    background: var(--accent-soft);
    box-shadow:
      inset 0 2px 0 var(--accent),
      inset 0 -2px 0 var(--accent);
  }
  .shade {
    position: absolute;
    top: 50%;
    height: 28px;
    margin-top: -14px;
    background: rgb(0 0 0 / 0.35);
    pointer-events: none;
  }
  .ghost {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: rgb(255 255 255 / 0.5);
  }
  .handle {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 36px;
    margin-top: -18px;
    background: var(--accent);
    cursor: ew-resize;
    z-index: 2;
  }
  .handle.start {
    margin-left: -12px;
    border-radius: var(--radius-sm) 0 0 var(--radius-sm);
  }
  .handle.end {
    border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
  }
  .handle::after {
    /* grip */
    content: "";
    position: absolute;
    left: 5px;
    top: 12px;
    width: 2px;
    height: 12px;
    border-radius: 1px;
    background: rgb(255 255 255 / 0.8);
  }
  .handle:hover {
    background: var(--accent-hover);
  }
  .head {
    position: absolute;
    top: 50%;
    width: 14px;
    height: 14px;
    margin: -7px 0 0 -7px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.5);
    transform: scale(0.85);
    transition: transform var(--t-fast) var(--ease);
    z-index: 3;
  }
  .trimming .head {
    width: 3px;
    height: 40px;
    margin: -20px 0 0 -1.5px;
    border-radius: 2px;
    transform: none;
  }
  .tl:hover .head,
  .tl.dragging .head,
  .head:focus-visible {
    transform: scale(1);
  }
  .tip {
    position: absolute;
    bottom: 100%;
    transform: translateX(-50%);
    margin-bottom: 4px;
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    pointer-events: none;
    z-index: 4;
  }
</style>
