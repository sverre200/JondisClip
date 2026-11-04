<script lang="ts">
  let {
    value,
    options,
    onchange,
    onopen,
    label,
    id,
    disabled = false,
  }: {
    value: string;
    options: { value: string; label: string }[];
    onchange: (v: string) => void;
    /** called when the user is about to open the list (refresh options) */
    onopen?: () => void;
    label: string;
    id?: string;
    disabled?: boolean;
  } = $props();
</script>

<div class="select">
  <select
    {id}
    aria-label={label}
    {disabled}
    {value}
    onfocus={onopen}
    onpointerdown={onopen}
    onchange={(e) => onchange((e.currentTarget as HTMLSelectElement).value)}
  >
    {#each options as o (o.value)}
      <option value={o.value}>{o.label}</option>
    {/each}
  </select>
  <svg class="chev" viewBox="0 0 16 16" aria-hidden="true">
    <path d="m4 6 4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
  </svg>
</div>

<style>
  .select {
    position: relative;
    display: inline-flex;
  }
  select {
    appearance: none;
    height: var(--control-h);
    min-width: 240px;
    max-width: 340px;
    padding: 0 calc(var(--s3) + 20px) 0 var(--s3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  select:hover:not(:disabled) {
    background: var(--surface-hover);
  }
  select:disabled {
    opacity: 0.5;
    cursor: default;
  }
  option {
    background: var(--surface-raised);
    color: var(--text);
  }
  .chev {
    position: absolute;
    right: var(--s3);
    top: 50%;
    width: 14px;
    height: 14px;
    transform: translateY(-50%);
    color: var(--text-muted);
    pointer-events: none;
  }
</style>
