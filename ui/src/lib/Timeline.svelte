<script lang="ts">
  import { clock } from "./format";

  // Seek bar. Stage 2 adds trim handles on top of the same track.
  let {
    duration,
    time,
    buffered = 0,
    onseek,
  }: {
    duration: number;
    time: number;
    /** seconds buffered ahead (for the faint fill) */
    buffered?: number;
    /** `fast` while dragging: the player may seek to the nearest keyframe */
    onseek: (t: number, fast?: boolean) => void;
  } = $props();

  let track = $state<HTMLDivElement>();
  let hover = $state<number | null>(null);
  let dragging = $state(false);

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

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    seekSoon(at(e));
  }
  function move(e: PointerEvent) {
    hover = at(e);
    if (dragging) seekSoon(hover);
  }
  function up(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    cancelAnimationFrame(frame);
    frame = 0;
    pending = null;
    onseek(at(e)); // exact position on release
  }

  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 1 / 60 : 5;
    let t: number | null = null;
    if (e.key === "ArrowRight") t = time + step;
    else if (e.key === "ArrowLeft") t = time - step;
    else if (e.key === "Home") t = 0;
    else if (e.key === "End") t = duration;
    if (t === null) return;
    e.preventDefault();
    e.stopPropagation();
    onseek(Math.min(duration, Math.max(0, t)));
  }
</script>

<div
  class="tl"
  class:dragging
  role="slider"
  tabindex="0"
  aria-label="Position"
  aria-valuemin={0}
  aria-valuemax={Math.round(duration)}
  aria-valuenow={Math.round(time)}
  aria-valuetext="{clock(time)} of {clock(duration)}"
  bind:this={track}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  onpointerleave={() => (hover = null)}
  onkeydown={key}
>
  <div class="rail">
    <div class="buf" style:width="{pct(buffered)}%"></div>
    <div class="fill" style:width="{pct(time)}%"></div>
    {#if hover !== null}<div class="ghost" style:left="{pct(hover)}%"></div>{/if}
  </div>
  <div class="head" style:left="{pct(time)}%"></div>
  {#if hover !== null}
    <div class="tip" style:left="{pct(hover)}%">{clock(hover)}</div>
  {/if}
</div>

<style>
  .tl {
    position: relative;
    height: 28px;
    cursor: pointer;
    touch-action: none;
    border-radius: var(--radius-sm);
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
  .ghost {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: rgb(255 255 255 / 0.5);
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
  }
  .tl:hover .head,
  .tl.dragging .head,
  .tl:focus-visible .head {
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
  }
</style>
