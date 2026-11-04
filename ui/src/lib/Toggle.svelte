<script lang="ts">
  let {
    checked,
    onchange,
    label,
    id,
    disabled = false,
  }: { checked: boolean; onchange: (v: boolean) => void; label: string; id?: string; disabled?: boolean } = $props();
</script>

<button
  {id}
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  class:on={checked}
  onclick={() => onchange(!checked)}
>
  <span class="thumb"></span>
</button>

<style>
  button {
    position: relative;
    width: 40px;
    height: 22px;
    flex: none;
    padding: 0;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    background: var(--surface-raised);
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease);
  }
  /* Bigger invisible hit area without changing the visual size. */
  button::after {
    content: "";
    position: absolute;
    inset: -8px 0;
  }
  button:hover:not(:disabled) {
    border-color: var(--text-faint);
  }
  .thumb {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--text-muted);
    transition:
      transform var(--t-med) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .on:hover:not(:disabled) {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }
  .on .thumb {
    transform: translateX(18px);
    background: var(--on-accent);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
