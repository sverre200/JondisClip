<script lang="ts">
  import { api, type MicCheck } from "./api";
  import Button from "./Button.svelte";
  import { t } from "./i18n.svelte";

  let {
    device,
    gainDb,
    lowCut,
    onUseGain,
  }: {
    device: string;
    gainDb: number;
    lowCut: boolean;
    /** apply a suggested JondisClip gain */
    onUseGain: (db: number) => void;
  } = $props();

  const SECONDS = 8;
  let phase = $state<"idle" | "recording" | "done" | "error">("idle");
  let left = $state(SECONDS);
  let result = $state<MicCheck | null>(null);
  let error = $state<string | null>(null);

  async function run() {
    phase = "recording";
    left = SECONDS;
    error = null;
    const timer = setInterval(() => (left = Math.max(0, left - 1)), 1000);
    try {
      result = await api.micCheck(device, gainDb, lowCut, SECONDS);
      phase = "done";
    } catch (e) {
      error = String(e);
      phase = "error";
    } finally {
      clearInterval(timer);
    }
  }

  const VERDICT = $derived({
    good: { title: t("Sounds good"), tone: "good" },
    quiet: { title: t("A bit quiet"), tone: "warn" },
    hot: { title: t("Too loud"), tone: "warn" },
    clipping: { title: t("The microphone is clipping"), tone: "bad" },
    no_speech: { title: t("Didn't hear you"), tone: "warn" },
  } as const);
  const fmt = (db: number) => (db <= -199 ? t("silent") : `${db.toFixed(0)} dBFS`);
  const showGainAction = $derived(
    !!result &&
      result.report.speech_detected &&
      result.report.adc_clipped === 0 &&
      Math.abs(result.report.suggested_gain_db - gainDb) >= 3 &&
      !result.report.knob_up,
  );
</script>

<div class="check">
  {#if phase === "recording"}
    <div class="live" role="status">
      <span class="dot" aria-hidden="true"></span>
      {t("Speak normally, like you do in game… {n} s", { n: left })}
    </div>
  {:else if phase === "done" && result}
    {@const r = result.report}
    {@const v = VERDICT[r.verdict]}
    <div class="result {v.tone}">
      <div class="head">
        <strong>{v.title}</strong>
        <Button variant="ghost" onclick={run}>{t("Test again")}</Button>
      </div>
      <dl>
        <div><dt>{t("Voice level")}</dt><dd>{fmt(r.speech_db)}</dd></div>
        <div><dt>{t("Peaks")}</dt><dd>{fmt(r.peak_db)}</dd></div>
        <div><dt>{t("Background noise")}</dt><dd>{fmt(r.noise_floor_db)}</dd></div>
        <div><dt>{t("Mic mode")}</dt><dd>{r.mode === "mono" ? "Mono" : r.mode === "two_track" ? "2-Track" : "Stereo"}</dd></div>
      </dl>
      <ul>
        {#each r.tips as tip, i (i)}<li>{tip}</li>{/each}
      </ul>
      <div class="actions">
        {#if result.audio}
          <!-- svelte-ignore a11y_media_has_caption -->
          <audio controls src={result.audio} aria-label={t("Listen back to the test recording")}></audio>
        {/if}
        {#if showGainAction}
          <Button onclick={() => onUseGain(r.suggested_gain_db)}>
            {t("Use {gain} dB", { gain: `${r.suggested_gain_db > 0 ? "+" : ""}${r.suggested_gain_db}` })}
          </Button>
        {/if}
      </div>
    </div>
  {:else}
    {#if phase === "error"}<p class="err" role="alert">{t("Couldn't record: {error}", { error: error ?? "" })}</p>{/if}
    <Button onclick={run}>{t("Test my mic ({n} s)", { n: SECONDS })}</Button>
  {/if}
</div>

<style>
  .check {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--s2);
    width: 100%;
  }
  .live {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    height: var(--control-h);
    padding: 0 var(--s4);
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    animation: blink 1s steps(2) infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.3;
    }
  }
  .result {
    width: 100%;
    padding: var(--s4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    border: 1px solid var(--border);
    border-left: 3px solid var(--text-faint);
  }
  .result.good {
    border-left-color: var(--success);
  }
  .result.warn {
    border-left-color: var(--warning);
  }
  .result.bad {
    border-left-color: var(--danger);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  dl {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--s3);
    margin: var(--s3) 0;
  }
  dt {
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  dd {
    margin: 2px 0 0;
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    font-variant-numeric: tabular-nums;
  }
  ul {
    margin: 0;
    padding-left: var(--s4);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    display: flex;
    flex-direction: column;
    gap: var(--s1);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--s3);
    margin-top: var(--s3);
  }
  audio {
    height: 32px;
    flex: 1;
    color-scheme: dark;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>
