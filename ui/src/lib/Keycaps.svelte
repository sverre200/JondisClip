<script lang="ts">
  import { keys } from "./format";

  let { hotkey, dim = false }: { hotkey: string; dim?: boolean } = $props();
</script>

<span class="caps" class:dim aria-label={hotkey}>
  {#each keys(hotkey) as k, i (i)}
    {#if i > 0}<span class="plus" aria-hidden="true">+</span>{/if}
    <kbd>{k}</kbd>
  {/each}
</span>

<style>
  .caps {
    display: inline-flex;
    align-items: center;
    gap: var(--s1);
  }
  kbd {
    min-width: 28px;
    height: 26px;
    padding: 0 var(--s2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font);
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: var(--radius-sm);
  }
  .plus {
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
  .dim kbd {
    color: var(--text-muted);
  }
</style>
