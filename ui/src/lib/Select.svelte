<script lang="ts">
  import { tick } from "svelte";

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

  // A themed list instead of the native <select> popup, which WebView2 draws
  // in the OS style (light grey highlight) whatever the theme.
  const uid = `sel-${Math.random().toString(36).slice(2, 8)}`;
  let trigger = $state<HTMLButtonElement>();
  let list = $state<HTMLDivElement>();
  let open = $state(false);
  let active = $state(0);
  let pos = $state({ left: 0, top: 0, width: 0, up: false, maxH: 320 });

  const current = $derived(options.find((o) => o.value === value));

  function place() {
    if (!trigger) return;
    const r = trigger.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - 8;
    const above = r.top - 8;
    const want = Math.min(options.length * 30 + 10, 320);
    const up = below < want && above > below;
    pos = { left: r.left, top: up ? r.top - 4 : r.bottom + 4, width: r.width, up, maxH: Math.max(120, Math.min(320, up ? above : below)) };
  }

  async function show() {
    if (disabled || open) return;
    onopen?.();
    active = Math.max(0, options.findIndex((o) => o.value === value));
    place();
    open = true;
    await tick();
    list?.focus();
    scrollActive();
  }

  function close(refocus = true) {
    if (!open) return;
    open = false;
    if (refocus) trigger?.focus();
  }

  function pick(i: number) {
    const o = options[i];
    close();
    if (o && o.value !== value) onchange(o.value);
  }

  function scrollActive() {
    list?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function move(to: number) {
    if (!options.length) return;
    active = Math.max(0, Math.min(options.length - 1, to));
    void tick().then(scrollActive);
  }

  let typed = "";
  let typedAt = 0;
  function typeAhead(ch: string) {
    const now = performance.now();
    typed = now - typedAt > 700 ? ch : typed + ch;
    typedAt = now;
    const q = typed.toLowerCase();
    const n = options.length;
    for (let k = 1; k <= n; k++) {
      const i = (active + (typed.length > 1 ? 0 : k)) % n;
      if (options[i]?.label.toLowerCase().startsWith(q)) return move(i);
    }
  }

  function onTriggerKey(e: KeyboardEvent) {
    if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key) || (e.altKey && e.key === "ArrowDown")) {
      e.preventDefault();
      void show();
    }
  }

  function onListKey(e: KeyboardEvent) {
    e.stopPropagation();
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        move(active + 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        move(active - 1);
        break;
      case "Home":
        e.preventDefault();
        move(0);
        break;
      case "End":
        e.preventDefault();
        move(options.length - 1);
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        pick(active);
        break;
      case "Escape":
        e.preventDefault();
        close();
        break;
      case "Tab":
        close(false);
        break;
      default:
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) typeAhead(e.key);
    }
  }

  // Close on a click elsewhere, a scroll outside the list, or a resize.
  $effect(() => {
    if (!open) return;
    const down = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!list?.contains(t) && !trigger?.contains(t)) close(false);
    };
    const scroll = (e: Event) => {
      if (!list?.contains(e.target as Node)) close(false);
    };
    const resize = () => close(false);
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("scroll", scroll, true);
    window.addEventListener("resize", resize);
    window.addEventListener("blur", resize);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("scroll", scroll, true);
      window.removeEventListener("resize", resize);
      window.removeEventListener("blur", resize);
    };
  });
</script>

<button
  bind:this={trigger}
  {id}
  type="button"
  class="trigger"
  class:open
  aria-label={label}
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-controls={open ? uid : undefined}
  {disabled}
  onclick={() => (open ? close() : void show())}
  onkeydown={onTriggerKey}
>
  <span class="value">{current?.label ?? ""}</span>
  <svg class="chev" viewBox="0 0 16 16" aria-hidden="true">
    <path d="m4 6 4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
  </svg>
</button>

{#if open}
  <div
    bind:this={list}
    id={uid}
    class="list"
    class:up={pos.up}
    role="listbox"
    tabindex="-1"
    aria-label={label}
    aria-activedescendant="{uid}-{active}"
    style:left="{pos.left}px"
    style:top="{pos.top}px"
    style:min-width="{pos.width}px"
    style:max-height="{pos.maxH}px"
    onkeydown={onListKey}
  >
    {#each options as o, i (o.value)}
      <div
        id="{uid}-{i}"
        data-i={i}
        class="opt"
        class:active={i === active}
        role="option"
        tabindex="-1"
        aria-selected={o.value === value}
        onpointermove={() => (active = i)}
        onclick={() => pick(i)}
        onkeydown={() => {}}
      >
        <svg class="check" class:on={o.value === value} viewBox="0 0 16 16" aria-hidden="true">
          <path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        <span class="olabel">{o.label}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .trigger {
    position: relative;
    display: inline-flex;
    align-items: center;
    height: var(--control-h);
    min-width: 240px;
    max-width: 340px;
    padding: 0 calc(var(--s3) + 20px) 0 var(--s3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  .trigger:hover:not(:disabled),
  .trigger.open {
    background: var(--surface-hover);
  }
  .trigger:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    transition: transform var(--t-fast) var(--ease);
  }
  .trigger.open .chev {
    transform: translateY(-50%) rotate(180deg);
  }
  /* Same look as the right-click menu. */
  .list {
    position: fixed;
    z-index: 60;
    max-width: 420px;
    overflow-y: auto;
    padding: var(--s1);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    outline: none;
    font-size: var(--fs-sm);
  }
  .list.up {
    transform: translateY(-100%);
  }
  .opt {
    display: flex;
    align-items: center;
    gap: var(--s2);
    height: 30px;
    padding: 0 var(--s3) 0 var(--s2);
    border-radius: var(--radius-sm);
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
  }
  .opt.active {
    background: var(--surface-hover);
  }
  .opt[aria-selected="true"] {
    font-weight: var(--fw-strong);
  }
  .olabel {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .check {
    flex: none;
    width: 14px;
    height: 14px;
    color: var(--accent);
    visibility: hidden;
  }
  .check.on {
    visibility: visible;
  }
</style>
