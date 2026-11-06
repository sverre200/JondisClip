<script lang="ts">
  import type { TrayView } from "./api";
  import { t } from "./i18n.svelte";

  let { tray }: { tray: TrayView | null } = $props();

  const view = $derived.by(() => {
    if (!tray) return { tone: "idle", text: t("Checking…") };
    if (!tray.running || !tray.status) return { tone: "off", text: t("Not running") };
    switch (tray.status.state) {
      case "recording":
        return { tone: "live", text: t("Recording") };
      case "paused":
        return { tone: "idle", text: t("Paused") };
      case "sleeping":
        return { tone: "idle", text: t("Sleeping") };
      case "low_disk":
        return { tone: "warn", text: t("Paused · low disk space") };
      default:
        return { tone: "warn", text: t("Restarting") };
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
    background: var(--recording-soft);
    border-color: transparent;
  }
  .live .dot {
    background: var(--recording);
    box-shadow: 0 0 0 3px var(--recording-glow);
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
