<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    variant = "secondary",
    onclick,
    disabled = false,
    title,
    children,
  }: {
    variant?: "primary" | "secondary" | "ghost";
    onclick?: (e: MouseEvent) => void;
    disabled?: boolean;
    title?: string;
    children: Snippet;
  } = $props();
</script>

<button type="button" class={variant} {onclick} {disabled} {title}>{@render children()}</button>

<style>
  button {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    height: var(--control-h);
    padding: 0 var(--s4);
    border-radius: var(--radius-md);
    border: 1px solid transparent;
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    white-space: nowrap;
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease);
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent);
  }
  .primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .primary:active:not(:disabled) {
    background: var(--accent-press);
  }
  .secondary {
    background: var(--surface-raised);
    border-color: var(--border-strong);
    color: var(--text);
  }
  .secondary:hover:not(:disabled) {
    background: var(--surface-hover);
  }
  .secondary:active:not(:disabled) {
    background: var(--surface-press);
  }
  .ghost {
    background: transparent;
    color: var(--text-muted);
    padding: 0 var(--s3);
  }
  .ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
