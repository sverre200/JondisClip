<script lang="ts">
  import Button from "./Button.svelte";
  import Keycaps from "./Keycaps.svelte";
  import { heldModifiers, hotkeyFromEvent } from "./format";

  let {
    value,
    conflict = false,
    validate,
    onchange,
  }: {
    value: string;
    /** the tray couldn't register this hotkey */
    conflict?: boolean;
    validate: (text: string) => Promise<string>;
    onchange: (canonical: string) => void;
  } = $props();

  let capturing = $state(false);
  let held = $state<string[]>([]);
  let error = $state<string | null>(null);
  let box = $state<HTMLDivElement | undefined>();

  function start() {
    error = null;
    held = [];
    capturing = true;
    queueMicrotask(() => box?.focus());
  }

  function stop() {
    capturing = false;
    held = [];
    error = null;
  }

  async function onkeydown(e: KeyboardEvent) {
    if (!capturing) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
      stop();
      return;
    }
    const combo = hotkeyFromEvent(e);
    if (!combo) {
      held = heldModifiers(e);
      return;
    }
    try {
      const canonical = await validate(combo);
      stop();
      if (canonical !== value) onchange(canonical);
    } catch (err) {
      error = String(err);
      held = [];
    }
  }

  function onkeyup(e: KeyboardEvent) {
    if (capturing) held = heldModifiers(e);
  }
</script>

<div class="field">
<div class="wrap">
  {#if capturing}
    <div
      bind:this={box}
      class="capture"
      role="textbox"
      tabindex="0"
      aria-label="Press the new hotkey. Escape cancels."
      {onkeydown}
      {onkeyup}
      onblur={stop}
    >
      {#if held.length}
        <Keycaps hotkey={held.join("+") + "+…"} dim />
      {:else}
        <span class="prompt">Press keys…</span>
      {/if}
    </div>
    <Button variant="ghost" onclick={stop}>Cancel</Button>
  {:else}
    <Keycaps hotkey={value} />
    <Button onclick={start}>Change</Button>
  {/if}
</div>
{#if error}
  <p class="msg error" role="alert">{error}</p>
{:else if capturing}
  <p class="msg">Use a modifier (Ctrl, Alt, Shift, Win) with a key, or a single F-key. Esc cancels.</p>
{:else if conflict}
  <p class="msg warn" role="alert">Another program is using {value}. Choose a different hotkey.</p>
{/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    max-width: 360px;
  }
  .wrap {
    display: flex;
    align-items: center;
    gap: var(--s2);
  }
  .capture {
    min-width: 180px;
    height: var(--control-h);
    padding: 0 var(--s3);
    display: flex;
    align-items: center;
    border-radius: var(--radius-md);
    border: 1px solid var(--accent);
    background: var(--accent-soft);
    outline: none;
  }
  .prompt {
    color: var(--text);
    font-size: var(--fs-sm);
    animation: pulse 1.4s var(--ease) infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.55;
    }
  }
  .msg {
    margin: var(--s1) 0 0;
    font-size: var(--fs-sm);
    color: var(--text-muted);
    text-align: right;
  }
  .error {
    color: var(--danger);
  }
  .warn {
    color: var(--warning);
  }
</style>
