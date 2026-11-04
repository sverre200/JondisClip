<script lang="ts">
  import { onMount } from "svelte";
  import { thumbUrl, type Clip } from "./api";
  import { bytes, clock, timeOfDay } from "./format";
  import { requestThumb } from "./thumbs";

  let {
    clip,
    selected,
    tabindex,
    onselect,
    onopen,
    el = $bindable(),
  }: {
    clip: Clip;
    selected: boolean;
    tabindex: number;
    onselect: () => void;
    onopen: () => void;
    el?: HTMLButtonElement;
  } = $props();

  // Reset only when the file changes (new id), not on every rescan, which
  // hands us new objects for the same clips.
  let ready = $state(false);
  let failed = $state(false);
  let seenId = "";
  $effect(() => {
    if (clip.id === seenId) {
      if (clip.thumb) ready = true;
      return;
    }
    seenId = clip.id;
    ready = clip.thumb;
    failed = false;
  });

  // Ask for a thumbnail only while the card is (nearly) on screen.
  onMount(() => {
    if (!el) return;
    let pending: { cancel: () => void } | null = null;
    const io = new IntersectionObserver(
      ([e]) => {
        if (!e) return;
        if (e.isIntersecting && !ready && !failed && !pending && clip.probe) {
          const r = requestThumb(clip);
          pending = r;
          void r.done.then((ok) => {
            pending = null;
            if (ok) ready = true;
            else failed = true;
          });
        } else if (!e.isIntersecting && pending) {
          pending.cancel();
          pending = null;
        }
      },
      { rootMargin: "200px" },
    );
    io.observe(el);
    return () => {
      io.disconnect();
      pending?.cancel();
    };
  });

  const res = $derived(clip.probe ? `${clip.probe.height}p` : null);
</script>

<button
  bind:this={el}
  type="button"
  class="card"
  class:selected
  {tabindex}
  aria-pressed={selected}
  aria-label="{clip.game}, {timeOfDay(clip.recorded)}, {clip.probe ? clock(clip.probe.duration) : 'unreadable'}"
  onclick={onselect}
  ondblclick={onopen}
>
  <div class="thumb">
    {#if clip.error}
      <div class="ph warn">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 2 20h20L12 3Zm0 6v5m0 3v.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
        <span>Can't read this clip</span>
      </div>
    {:else if ready}
      <img src={thumbUrl(clip.id)} alt="" decoding="async" loading="lazy" draggable="false" />
    {:else}
      <div class="ph" class:loading={!failed}>
        {#if failed}
          <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2" fill="none" stroke="currentColor" stroke-width="1.6" /><path d="m10 9.5 5 2.5-5 2.5z" fill="currentColor" /></svg>
        {/if}
      </div>
    {/if}
    {#if clip.probe}<span class="dur">{clock(clip.probe.duration)}</span>{/if}
    {#if clip.edit}<span class="tag">Edit</span>{/if}
  </div>
  <div class="meta">
    <div class="l1">
      <span class="game">{clip.game}</span>
      <span class="time">{timeOfDay(clip.recorded)}</span>
    </div>
    <div class="l2">
      {#if res}{res} · {/if}{bytes(clip.size)}
    </div>
  </div>
</button>

<style>
  .card {
    display: flex;
    flex-direction: column;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    text-align: left;
    cursor: pointer;
    overflow: hidden;
    transition:
      border-color var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .card:hover {
    background: var(--surface-raised);
    border-color: var(--border-strong);
  }
  .card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
    background: var(--surface-raised);
  }
  .card:focus-visible {
    outline-offset: 3px;
  }
  /* The selection ring already shows where focus is. */
  .card.selected:focus-visible {
    outline: none;
  }
  .thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    background: #000;
  }
  img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .ph {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s2);
    background: var(--surface-raised);
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
  .ph svg {
    width: 28px;
    height: 28px;
  }
  .ph.warn {
    color: var(--warning);
  }
  /* Opacity only (compositor), and only while a thumbnail is on its way. */
  .ph.loading {
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.55;
    }
  }
  .dur {
    position: absolute;
    right: var(--s2);
    bottom: var(--s2);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: rgb(0 0 0 / 0.72);
    color: #fff;
    font-size: var(--fs-xs);
    font-weight: var(--fw-strong);
    font-variant-numeric: tabular-nums;
  }
  .tag {
    position: absolute;
    left: var(--s2);
    top: var(--s2);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
    font-size: var(--fs-xs);
    font-weight: var(--fw-strong);
  }
  .meta {
    padding: var(--s2) var(--s3) var(--s3);
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .l1 {
    display: flex;
    justify-content: space-between;
    gap: var(--s2);
    font-size: var(--fs-sm);
  }
  .game {
    font-weight: var(--fw-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .time {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    flex: none;
  }
  .l2 {
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
</style>
