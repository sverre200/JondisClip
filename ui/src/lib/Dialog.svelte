<script lang="ts">
  import { onMount, tick, type Snippet } from "svelte";
  import Button from "./Button.svelte";

  // A small modal for confirmations and name prompts. Enter confirms, Esc
  // cancels; focus stays inside while it's open.
  let {
    title,
    confirmLabel = "OK",
    danger = false,
    input = null,
    error = null,
    children,
    onconfirm,
    oncancel,
  }: {
    title: string;
    confirmLabel?: string;
    danger?: boolean;
    /** show a text field (prompt) with this initial value */
    input?: { value: string; placeholder?: string; max?: number } | null;
    error?: string | null;
    children?: Snippet;
    onconfirm: (value: string) => void;
    oncancel: () => void;
  } = $props();

  let value = $state("");
  let field = $state<HTMLInputElement>();
  let box = $state<HTMLDivElement>();

  onMount(() => {
    value = input?.value ?? "";
    void tick().then(() => {
      if (field) {
        field.focus();
        field.select();
      } else box?.querySelector<HTMLButtonElement>(danger ? ".buttons button" : "button.primary")?.focus();
    });
  });

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      oncancel();
    } else if (e.key === "Enter" && !danger && !(e.target instanceof HTMLButtonElement)) {
      e.preventDefault();
      onconfirm(value);
    } else if (e.key === "Tab" && box) {
      // Keep focus in the dialog.
      const f = [...box.querySelectorAll<HTMLElement>("input, button:not(:disabled)")];
      const first = f[0];
      const last = f[f.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first?.focus();
      }
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onpointerdown={(e) => e.target === e.currentTarget && oncancel()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title} bind:this={box} tabindex="-1" onkeydown={key}>
    <h2>{title}</h2>
    {#if children}<div class="body">{@render children()}</div>{/if}
    {#if input}
      <input
        bind:this={field}
        bind:value
        type="text"
        maxlength={input.max ?? 120}
        placeholder={input.placeholder ?? ""}
        spellcheck="false"
        aria-label={title}
        aria-invalid={!!error}
      />
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="buttons">
      <Button variant="ghost" onclick={oncancel}>Cancel</Button>
      <Button variant="primary" onclick={() => onconfirm(value)}>{confirmLabel}</Button>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.45);
  }
  .dialog {
    width: min(400px, calc(100vw - 48px));
    padding: var(--s5);
    border-radius: var(--radius-lg);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    outline: none;
  }
  h2 {
    margin: 0 0 var(--s2);
    font-size: var(--fs-md);
    font-weight: var(--fw-strong);
  }
  .body {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    margin-bottom: var(--s3);
  }
  .body :global(p) {
    margin: 0 0 var(--s2);
  }
  input {
    width: 100%;
    height: var(--control-h);
    padding: 0 var(--s3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--surface);
    font-size: var(--fs-sm);
    outline: none;
    user-select: text;
  }
  input:focus {
    border-color: var(--text-faint);
  }
  input[aria-invalid="true"] {
    border-color: var(--danger);
  }
  .err {
    margin: var(--s2) 0 0;
    color: var(--danger);
    font-size: var(--fs-xs);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: var(--s2);
    margin-top: var(--s4);
  }
</style>
