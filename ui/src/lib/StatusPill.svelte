<script lang="ts">
  import type { TrayView } from "./api";

  let { tray }: { tray: TrayView | null } = $props();

  const view = $derived.by(() => {
    if (!tray) return { tone: "idle", text: "Checking…" };
    if (!tray.running || !tray.status) return { tone: "off", text: "Not running" };
    switch (tray.status.state) {
      case "recording":
        return { tone: "live", text: "Recording" };
      case "paused":
        return { tone: "idle", text: "Paused" };
      case "sleeping":
        return { tone: "idle", text: "Sleeping" };
      case "low_disk":
        return { tone: "warn", text: "Paused · low disk space" };
      default:
        return { tone: "warn", text: "Restarting" };
    }
  });
</script>

<span class="pill {view.tone}" role="status" title={tray?.status?.summary ?? ""}>
  <span class="dot" aria-hidden="true"></span>{view.text}
</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    height: 28px;
    padding: 0 var(--s3);
    border-radius: var(--radius-pill);
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-faint);
  }
  .live {
    color: var(--text);
    background: var(--accent-soft);
    border-color: transparent;
  }
  .live .dot {
    background: var(--accent);
    box-shadow: 0 0 0 3px rgb(229 72 77 / 0.25);
  }
  .warn {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: transparent;
  }
  .warn .dot {
    background: var(--warning);
  }
  .off .dot {
    background: transparent;
    border: 1.5px solid var(--text-faint);
  }
</style>
