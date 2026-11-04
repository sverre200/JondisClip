<script lang="ts" module>
  export interface MenuItem {
    label: string;
    action?: () => void;
    shortcut?: string;
    danger?: boolean;
    disabled?: boolean;
    /** a small colour dot before the label (folder colours) */
    dot?: string;
    /** a check mark (current choice) */
    checked?: boolean;
    submenu?: MenuItem[];
  }
  export type MenuEntry = MenuItem | "separator";
</script>

<script lang="ts">
  import { onMount, tick } from "svelte";

  // One context menu: Windows-like, positioned at the pointer and kept on
  // screen, keyboard navigable (arrows, Enter, Esc, → for submenus).
  let { x, y, items, onclose }: { x: number; y: number; items: MenuEntry[]; onclose: () => void } = $props();

  let el = $state<HTMLDivElement>();
  let pos = $state({ left: 0, top: 0 });
  let active = $state(-1);
  let openSub = $state<number | null>(null);
  // Submenus open to the left when there's no room on the right.
  let flip = $state(false);

  const actionable = $derived(
    items.map((it, i) => (it !== "separator" && !it.disabled ? i : -1)).filter((i) => i >= 0),
  );

  onMount(() => {
    void (async () => {
      await tick();
      if (!el) return;
      const r = el.getBoundingClientRect();
      pos = {
        left: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)),
        top: Math.max(4, Math.min(y, window.innerHeight - r.height - 4)),
      };
      flip = pos.left + r.width + 230 > window.innerWidth;
      el.focus();
    })();
    const away = (e: Event) => {
      if (el && !el.contains(e.target as Node)) onclose();
    };
    const blur = () => onclose();
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("blur", blur);
    window.addEventListener("resize", blur);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("blur", blur);
      window.removeEventListener("resize", blur);
    };
  });

  function run(it: MenuItem) {
    if (it.disabled) return;
    if (it.submenu) return;
    onclose();
    it.action?.();
  }

  function key(e: KeyboardEvent) {
    const pos = actionable.indexOf(active);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (openSub !== null) openSub = null;
      else onclose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      active = actionable[(pos + 1) % actionable.length] ?? -1;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = actionable[(pos - 1 + actionable.length) % actionable.length] ?? -1;
    } else if (e.key === "ArrowRight" || e.key === "Enter" || e.key === " ") {
      const it = items[active];
      if (!it || it === "separator") return;
      e.preventDefault();
      if (it.submenu) openSub = active;
      else if (e.key !== "ArrowRight") run(it);
    } else if (e.key === "ArrowLeft") {
      openSub = null;
    }
  }
</script>

<div
  class="menu"
  role="menu"
  tabindex="-1"
  bind:this={el}
  style:left="{pos.left}px"
  style:top="{pos.top}px"
  onkeydown={key}
  oncontextmenu={(e) => e.preventDefault()}
>
  {#each items as it, i (i)}
    {#if it === "separator"}
      <div class="sep" role="separator"></div>
    {:else}
      <div class="wrap">
        <button
          type="button"
          role="menuitem"
          class="item"
          class:danger={it.danger}
          class:active={active === i}
          disabled={it.disabled}
          aria-haspopup={it.submenu ? "menu" : undefined}
          aria-expanded={it.submenu ? openSub === i : undefined}
          onpointerenter={() => {
            active = i;
            openSub = it.submenu ? i : null;
          }}
          onclick={() => (it.submenu ? (openSub = i) : run(it))}
        >
          <span class="check">{#if it.checked}✓{/if}</span>
          {#if it.dot}<span class="dot" style:background="var(--folder-{it.dot})"></span>{/if}
          <span class="label">{it.label}</span>
          {#if it.shortcut}<span class="kbd">{it.shortcut}</span>{/if}
          {#if it.submenu}
            <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="m6 4 4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>
          {/if}
        </button>
        {#if it.submenu && openSub === i}
          <div class="menu sub" class:flip role="menu">
            {#each it.submenu as sub, j (j)}
              <button type="button" role="menuitem" class="item" disabled={sub.disabled} onclick={() => run(sub)}>
                <span class="check">{#if sub.checked}✓{/if}</span>
                {#if sub.dot}<span class="dot" style:background="var(--folder-{sub.dot})"></span>{/if}
                <span class="label">{sub.label}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 50;
    min-width: 200px;
    max-width: 280px;
    padding: var(--s1);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    outline: none;
    font-size: var(--fs-sm);
  }
  .wrap {
    position: relative;
  }
  .sub.flip {
    left: auto;
    right: calc(100% + 2px);
  }
  .sub {
    position: absolute;
    left: calc(100% + 2px);
    top: calc(-1 * var(--s1));
    max-height: 60vh;
    overflow-y: auto;
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--s2);
    width: 100%;
    height: 30px;
    padding: 0 var(--s2) 0 var(--s1);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .item:hover:not(:disabled),
  .item.active:not(:disabled) {
    background: var(--surface-hover);
  }
  .item:disabled {
    color: var(--text-faint);
    cursor: default;
  }
  .item.danger {
    color: var(--danger);
  }
  .check {
    width: 14px;
    flex: none;
    text-align: center;
    color: var(--text-muted);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kbd {
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
  .chev {
    width: 12px;
    height: 12px;
    color: var(--text-faint);
  }
  .sep {
    height: 1px;
    margin: var(--s1) var(--s2);
    background: var(--border-strong);
  }
</style>
