<script lang="ts">
  let {
    value,
    min,
    max,
    step = 1,
    label,
    id,
    format,
    oninput,
    onchange,
  }: {
    value: number;
    min: number;
    max: number;
    step?: number;
    label: string;
    id?: string;
    format: (v: number) => string;
    /** while dragging (live preview) */
    oninput?: (v: number) => void;
    /** on release / keyboard commit (save) */
    onchange: (v: number) => void;
  } = $props();

  const pct = $derived(((value - min) / (max - min)) * 100);
</script>

<div class="slider">
  <input
    {id}
    type="range"
    {min}
    {max}
    {step}
    {value}
    aria-label={label}
    aria-valuetext={format(value)}
    style="--pct: {pct}%"
    oninput={(e) => oninput?.(Number((e.currentTarget as HTMLInputElement).value))}
    onchange={(e) => onchange(Number((e.currentTarget as HTMLInputElement).value))}
  />
  <output for={id}>{format(value)}</output>
</div>

<style>
  .slider {
    display: flex;
    align-items: center;
    gap: var(--s3);
  }
  output {
    min-width: 56px;
    text-align: right;
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    font-variant-numeric: tabular-nums;
  }
  input {
    appearance: none;
    width: 200px;
    height: var(--control-h);
    margin: 0;
    background: transparent;
    cursor: pointer;
  }
  input::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: var(--radius-pill);
    background: linear-gradient(to right, var(--accent) 0 var(--pct), var(--border-strong) var(--pct) 100%);
  }
  input::-webkit-slider-thumb {
    appearance: none;
    width: 16px;
    height: 16px;
    margin-top: -6px;
    border-radius: 50%;
    background: var(--text);
    border: 2px solid var(--bg);
    box-shadow: 0 0 0 1px var(--border-strong);
    transition: transform var(--t-fast) var(--ease);
  }
  input:hover::-webkit-slider-thumb {
    transform: scale(1.1);
  }
  input:focus-visible {
    outline: none;
  }
  input:focus-visible::-webkit-slider-thumb {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
</style>
