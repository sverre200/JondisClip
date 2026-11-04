<script lang="ts" generics="T extends string | number">
  let {
    options,
    value,
    onchange,
    label,
    id,
    disabled = false,
  }: {
    options: { value: T; label: string; title?: string }[];
    value: T;
    onchange: (v: T) => void;
    label: string;
    id?: string;
    disabled?: boolean;
  } = $props();

  let buttons = $state<HTMLButtonElement[]>([]);

  function onkeydown(e: KeyboardEvent, i: number) {
    const dir = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!dir) return;
    e.preventDefault();
    const next = (i + dir + options.length) % options.length;
    const opt = options[next];
    if (opt) {
      onchange(opt.value);
      buttons[next]?.focus();
    }
  }
</script>

<div class="seg" role="radiogroup" aria-label={label} {id}>
  {#each options as opt, i (opt.value)}
    <button
      bind:this={buttons[i]}
      type="button"
      role="radio"
      aria-checked={opt.value === value}
      tabindex={opt.value === value ? 0 : -1}
      aria-label={opt.label}
      title={opt.title}
      {disabled}
      class:on={opt.value === value}
      onclick={() => onchange(opt.value)}
      onkeydown={(e) => onkeydown(e, i)}>{opt.label}</button
    >
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    padding: 3px;
    gap: 2px;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }
  button {
    height: calc(var(--control-h) - 8px);
    min-width: 44px;
    padding: 0 var(--s3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: var(--fw-regular);
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease);
  }
  button:hover:not(.on):not(:disabled) {
    background: var(--surface-hover);
    color: var(--text);
  }
  button:active:not(:disabled) {
    background: var(--surface-press);
  }
  button.on {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: var(--fw-strong);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
