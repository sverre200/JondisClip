<script lang="ts">
  import type { Snippet } from "svelte";

  let { tone = "warn", children, action }: { tone?: "warn" | "error"; children: Snippet; action?: Snippet } = $props();
</script>

<div class="banner {tone}" role={tone === "error" ? "alert" : "status"}>
  <svg class="icon" viewBox="0 0 16 16" aria-hidden="true">
    <path d="M8 1.5 15 14H1L8 1.5Z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" />
    <path d="M8 6v3.5M8 11.6v.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
  </svg>
  <div class="body">{@render children()}</div>
  {#if action}<div class="action">{@render action()}</div>{/if}
</div>

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: var(--s3) var(--s4);
    border-radius: var(--radius-lg);
    font-size: var(--fs-sm);
  }
  .warn {
    background: var(--warning-soft);
    color: var(--text);
  }
  .warn .icon {
    color: var(--warning);
  }
  .error {
    background: var(--danger-soft);
  }
  .error .icon {
    color: var(--danger);
  }
  .icon {
    width: 16px;
    height: 16px;
    flex: none;
  }
  .body {
    flex: 1;
  }
  .action {
    flex: none;
  }
</style>
