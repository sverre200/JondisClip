<script lang="ts">
  import { t } from "./i18n.svelte";
  // Horizontal mic meter, -60..0 dBFS. Zones guide the gain setting: speech
  // peaks should land in "good" (-18..-6 dBFS). Above -3 dBFS the limiter will
  // catch peaks; consistently there means the gain is too high.
  let { peakDb, rmsDb, hold }: { peakDb: number; rmsDb: number; hold: number } = $props();

  const MIN = -60;
  const pos = (db: number) => Math.min(100, Math.max(0, ((db - MIN) / -MIN) * 100));
  // Colour follows the 1.5 s peak hold (same as the hint text), so it doesn't flicker.
  const zone = $derived(hold > -3 ? "hot" : hold > -18 ? "good" : hold > -40 ? "low" : "idle");
</script>

<div
  class="meter"
  role="meter"
  aria-label={t("Microphone level")}
  aria-valuemin={MIN}
  aria-valuemax={0}
  aria-valuenow={Math.round(peakDb)}
  aria-valuetext={peakDb <= -99 ? t("silence") : `${Math.round(peakDb)} dB`}
>
  <div class="track">
    <span class="z good" style="left:{pos(-18)}%; width:{pos(-6) - pos(-18)}%"></span>
    <span class="z hot" style="left:{pos(-3)}%; width:{100 - pos(-3)}%"></span>
    <span class="rms {zone}" style="width:{pos(rmsDb)}%"></span>
    <span class="peak {zone}" style="width:{pos(peakDb)}%"></span>
    {#if hold > MIN}<span class="hold" style="left:calc({pos(hold)}% - 1px)"></span>{/if}
  </div>
  <div class="scale" aria-hidden="true">
    <span style="left:0">-60</span><span style="left:{pos(-18)}%">-18</span><span
      style="left:100%">0 dB</span
    >
  </div>
</div>

<style>
  .meter {
    width: 300px;
  }
  .track {
    position: relative;
    height: 10px;
    border-radius: var(--radius-pill);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .z {
    position: absolute;
    top: 0;
    bottom: 0;
    opacity: 0.35;
  }
  .z.good {
    background: var(--success-soft);
    opacity: 1;
  }
  .z.hot {
    background: var(--accent-soft);
    opacity: 1;
  }
  .peak,
  .rms {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    transition: width 80ms linear;
  }
  .peak {
    opacity: 0.45;
  }
  .rms,
  .peak {
    background: var(--text-faint);
  }
  .good {
    background: var(--success);
  }
  .low {
    background: var(--warning);
  }
  .hot {
    background: var(--accent);
  }
  .idle {
    background: var(--text-faint);
  }
  .hold {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--text);
  }
  .scale {
    position: relative;
    height: 14px;
    margin-top: 2px;
    font-size: 10px;
    color: var(--text-faint);
  }
  .scale span {
    position: absolute;
    transform: translateX(-50%);
    white-space: nowrap;
  }
  .scale span:first-child {
    transform: none;
  }
  .scale span:last-child {
    transform: translateX(-100%);
  }
</style>
