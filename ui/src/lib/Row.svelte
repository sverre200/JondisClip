<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    label,
    hint,
    id,
    stacked = false,
    hintLines = 0,
    children,
    below,
  }: {
    label: string;
    hint?: string;
    /** id of the control, for label association */
    id?: string;
    /** put the control under the text (wide controls) */
    stacked?: boolean;
    /** keep room for this many lines of hint, for hints whose text changes while you watch */
    hintLines?: number;
    children: Snippet;
    below?: Snippet;
  } = $props();
</script>

<div class="row" class:stacked>
  <div class="text">
    {#if id}<label class="label" for={id}>{label}</label>{:else}<div class="label">{label}</div>{/if}
    {#if hint}<div class="hint" style:min-height={hintLines ? `${hintLines}lh` : undefined}>{hint}</div>{/if}
  </div>
  <div class="control">{@render children()}</div>
  {#if below}<div class="below">{@render below()}</div>{/if}
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s2) var(--s5);
    padding: var(--s3) 0;
    border-top: 1px solid var(--border);
  }
  .row:first-of-type {
    border-top: none;
  }
  .row.stacked {
    grid-template-columns: minmax(0, 1fr);
  }
  .label {
    font-weight: var(--fw-strong);
    font-size: var(--fs-md);
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    margin-top: 2px;
  }
  .control {
    display: flex;
    align-items: center;
    gap: var(--s2);
    justify-self: end;
  }
  .stacked .control {
    justify-self: stretch;
  }
  .below {
    grid-column: 1 / -1;
  }
</style>
