<script lang="ts">
  import { drag } from "./drag.svelte";

  // What follows the pointer during a drag: a small thumbnail and a label.
  const p = $derived(drag.payload);
  const ok = $derived(!!drag.target);
</script>

{#if p}
  <div class="ghost" class:ok style:transform="translate({drag.x + 14}px, {drag.y + 14}px)" aria-hidden="true">
    {#if p.kind === "clips"}
      {#if p.thumb}<img src={p.thumb} alt="" />{:else}<span class="ph"></span>{/if}
    {:else}
      <span class="dot" style:background="var(--folder-{p.color})"></span>
    {/if}
    <span class="label">{p.label}</span>
  </div>
  {#if drag.hint}
    <div class="hint" style:transform="translate({drag.x + 14}px, {drag.y + 54}px)">{drag.hint}</div>
  {/if}
  {#if drag.target?.type === "insert"}
    <div class="bar" style:left="{drag.target.rect.left}px" style:top="{drag.target.rect.top}px" style:height="{drag.target.rect.height}px"></div>
  {/if}
{/if}

<style>
  .ghost,
  .hint {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 70;
    pointer-events: none;
  }
  .ghost {
    display: flex;
    align-items: center;
    gap: var(--s2);
    max-width: 240px;
    padding: 4px var(--s3) 4px 4px;
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    opacity: 0.92;
  }
  .ghost.ok {
    box-shadow: var(--shadow-pop), inset 0 0 0 1px var(--accent);
  }
  img,
  .ph {
    width: 48px;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-sm);
    object-fit: cover;
    background: var(--surface-hover);
    flex: none;
  }
  .dot {
    width: 10px;
    height: 10px;
    margin-left: var(--s2);
    border-radius: 50%;
    flex: none;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    padding: 2px var(--s2);
    border-radius: var(--radius-sm);
    background: var(--surface-raised);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    box-shadow: var(--shadow-pop);
  }
  .bar {
    position: fixed;
    z-index: 65;
    width: 3px;
    border-radius: 2px;
    background: var(--accent);
    pointer-events: none;
  }
</style>
