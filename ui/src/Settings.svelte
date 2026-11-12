<script lang="ts">
  import { onMount } from "svelte";
  import { api, isDesktop, type Codec, type DriveCheck, type InputDevice, type Quality, type Resolution, type Settings, type Snapshot, type TrayView } from "./lib/api";
  import { bytes, estimateBitrate, outputSize, replayLabel, replayShort } from "./lib/format";
  import Banner from "./lib/Banner.svelte";
  import Button from "./lib/Button.svelte";
  import HotkeyField from "./lib/HotkeyField.svelte";
  import Row from "./lib/Row.svelte";
  import Segmented from "./lib/Segmented.svelte";
  import LevelMeter from "./lib/LevelMeter.svelte";
  import MicCheck from "./lib/MicCheck.svelte";
  import Select from "./lib/Select.svelte";
  import Slider from "./lib/Slider.svelte";
  import Toggle from "./lib/Toggle.svelte";
  import { THEMES, applyTheme } from "./lib/theme";
  import { LANGUAGES, setLocale, t } from "./lib/i18n.svelte";

  // Tray status is polled by the shell (App.svelte) for both pages.
  let { tray }: { tray: TrayView | null } = $props();

  let snap = $state<Snapshot | null>(null);
  let settings = $state<Settings | null>(null);
  let loadError = $state<string | null>(null);
  let toast = $state<{ text: string; tone: "ok" | "error" } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let mics = $state<InputDevice[] | null>(null);
  // Gain while the slider is being dragged (saved on release).
  let gainPreview = $state<number | null>(null);
  let level = $state({ peak: -100, rms: -100, hold: -100, holdAt: 0, error: null as string | null });
  let micsError = $state<string | null>(null);

  async function refreshMics() {
    try {
      mics = await api.listMicrophones();
      micsError = null;
    } catch (e) {
      micsError = String(e);
    }
  }

  // Source resolution for size estimates: the screen in physical pixels,
  // falling back to 1080p if the webview reports nothing usable.
  const screenW = Math.round(window.screen.width * window.devicePixelRatio);
  const screenH = Math.round(window.screen.height * window.devicePixelRatio);
  const srcW = screenW >= 640 ? screenW : 1920;
  const srcH = screenH >= 360 ? screenH : 1080;

  function showToast(text: string, tone: "ok" | "error" = "ok") {
    toast = { text, tone };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), tone === "ok" ? 1600 : 4000);
  }

  async function change(patch: Partial<Settings>) {
    if (!settings) return;
    const before = settings;
    settings = { ...settings, ...patch }; // optimistic
    try {
      settings = await api.update(patch);
      showToast(tray?.running ? t("Saved · applied within 2 s") : t("Saved"));
    } catch (e) {
      settings = before;
      showToast(String(e), "error");
    }
  }

  /** Applies at once (only colours change, the recorder isn't involved), then saves. */
  async function setTheme(id: string) {
    if (!settings || settings.theme === id) return;
    const before = settings.theme;
    applyTheme(id);
    await change({ theme: id });
    if (settings.theme !== id) applyTheme(before);
  }

  /** Language and date format apply at once in the window, then save (the tray follows within 2 s). */
  async function setLanguage(lang: string, dateFormat: string) {
    if (!settings) return;
    const before = { language: settings.language, date_format: settings.date_format };
    setLocale(lang, dateFormat);
    await change({ language: lang, date_format: dateFormat });
    if (settings.language !== lang || settings.date_format !== dateFormat) setLocale(before.language, before.date_format);
  }
  const sample = new Date(2026, 10, 6);
  const dateFormats = $derived.by(() => {
    const pad = (n: number) => String(n).padStart(2, "0");
    const [y, m, d] = [sample.getFullYear(), pad(sample.getMonth() + 1), pad(sample.getDate())];
    return [
      { value: "dmy", label: `${d}.${m}.${y}`, title: t("Day.month.year") },
      { value: "ymd", label: `${y}-${m}-${d}`, title: t("Year-month-day") },
      { value: "mdy", label: `${m}/${d}/${y}`, title: t("Month/day/year") },
    ];
  });

  async function pickFolder() {
    if (!settings || !snap) return;
    const picked = await api.pickFolder(settings.save_dir || snap.default_save_dir);
    if (picked) await change({ save_dir: picked });
  }

  onMount(() => {
    (async () => {
      try {
        snap = await api.load();
        settings = snap.settings;
        void refreshMics();
      } catch (e) {
        loadError = String(e);
      }
    })();
    return () => void api.micMeterStop();
  });

  const saveDir = $derived(settings && snap ? settings.save_dir || snap.default_save_dir : "");
  const est = $derived.by(() => {
    if (!settings) return null;
    const bps = estimateBitrate(settings, srcW, srcH);
    const out = outputSize(settings, srcW, srcH);
    return {
      perMin: bytes((bps / 8) * 60),
      perClip: bytes((bps / 8) * settings.replay_seconds),
      out: `${out.w}×${out.h}`,
    };
  });
  const micOptions = $derived.by(() => {
    const list = mics ?? [];
    const def = list.find((m) => m.is_default);
    const opts = [{ value: "", label: def ? t("Windows default · {name}", { name: def.name }) : t("Windows default") }];
    for (const m of list) opts.push({ value: m.id, label: m.name });
    const chosen = settings?.microphone_device;
    if (chosen && mics && !list.some((m) => m.id === chosen)) {
      opts.push({ value: chosen, label: t("Selected microphone (not connected)") });
    }
    return opts;
  });
  const activeMic = $derived.by(() => {
    const chosen = settings?.microphone_device;
    return mics?.find((m) => (chosen ? m.id === chosen : m.is_default)) ?? null;
  });
  const micHint = $derived.by(() => {
    if (micsError) return t("Couldn't list microphones: {error}", { error: micsError });
    const chosen = settings?.microphone_device;
    if (chosen && mics && !mics.some((m) => m.id === chosen))
      return t("Not connected. The mic track stays silent until it's plugged back in.");
    if (activeMic?.is_virtual)
      return t("{name} is a virtual device, not a physical microphone. Choose your real microphone unless you route your voice through it on purpose.", { name: activeMic.name });
    if (!chosen) return t("Follows the Windows default if you change it.");
    return t("Always records this microphone, even if the Windows default changes.");
  });
  const gainDb = $derived(gainPreview ?? settings?.microphone_gain_db ?? 0);
  const levelHint = $derived.by(() => {
    if (level.error) return t("Can't read this microphone: {error}", { error: level.error });
    if (level.hold <= -60) return t("Speak normally: your voice should reach the green zone.");
    if (level.hold > -3) return t("Peaks are very hot. Lower the gain a little (loud peaks are limited, not clipped).");
    if (level.hold <= -18) return t("Quiet. Raise the gain until normal speech reaches the green zone.");
    return t("Good level.");
  });

  // Live meter: polls only while the mic is enabled; the backend capture stops
  // on its own once polling stops (window closed or mic switched off).
  const micOn = $derived(settings?.microphone ?? false);
  const micDevice = $derived(settings?.microphone_device ?? "");
  $effect(() => {
    if (!micOn) return;
    const device = micDevice;
    const timer = setInterval(async () => {
      try {
        const l = await api.micLevel(device, gainDb);
        const now = Date.now();
        const hold = l.peak_db >= level.hold || now - level.holdAt > 1500 ? l.peak_db : level.hold;
        level = { peak: l.peak_db, rms: l.rms_db, hold, holdAt: hold === l.peak_db ? now : level.holdAt, error: l.error };
      } catch {
        /* transient */
      }
    }, 100);
    return () => {
      clearInterval(timer);
      void api.micMeterStop();
      level = { peak: -100, rms: -100, hold: -100, holdAt: 0, error: null };
    };
  });
  const hotkeyConflict = $derived(
    !!tray?.running && !!tray.status && !tray.status.hotkey_ok && tray.status.hotkey === settings?.hotkey,
  );

  let driveSuggestion = $state<string | null>(null);
  onMount(() => {
    void api
      .driveStatus()
      .then((d) => (driveSuggestion = d.suggestions[0] ?? null))
      .catch(() => {});
  });
  // Drive folder health: checked when Settings opens, after a change, and on demand.
  let driveCheck = $state<DriveCheck | null>(null);
  let driveChecking = $state(false);
  async function checkDrive() {
    driveChecking = true;
    try {
      driveCheck = await api.driveCheck();
    } catch (e) {
      driveCheck = { dir: settings?.drive_dir ?? "", state: "problem", message: String(e) };
    } finally {
      driveChecking = false;
    }
  }
  onMount(() => void checkDrive());
  async function setDrive(dir: string) {
    await change({ drive_dir: dir });
    if (settings?.drive_dir === dir) await checkDrive();
  }
  async function pickDrive() {
    if (!settings) return;
    const picked = await api.pickDriveFolder(settings.drive_dir || driveSuggestion || "");
    if (picked) await setDrive(picked);
  }

  function discordLimits(current: number) {
    const presets = [
      { value: "10", label: "10 MB", title: t("Very strict servers or older limits") },
      { value: "19", label: "19 MB", title: t("Free account (recommended)") },
      { value: "50", label: "50 MB", title: "Nitro Basic" },
      { value: "500", label: "500 MB", title: "Nitro" },
    ];
    // A custom value from the settings file stays visible.
    if (!presets.some((p) => p.value === String(current))) presets.push({ value: String(current), label: `${current} MB`, title: t("Set in settings.toml") });
    return presets;
  }

  const resolutions: { value: Resolution; label: string }[] = $derived([
    { value: "source", label: t("Native") },
    { value: "1440p", label: "1440p" },
    { value: "1080p", label: "1080p" },
    { value: "720p", label: "720p" },
  ]);
  const qualities: { value: Quality; label: string; title: string }[] = $derived([
    { value: "performance", label: t("Performance"), title: t("Smallest files, lightest on the GPU") },
    { value: "balanced", label: t("Balanced"), title: t("Recommended") },
    { value: "high", label: t("High"), title: t("Sharpest motion, biggest files") },
  ]);
  const codecs: { value: Codec; label: string; title: string }[] = $derived([
    { value: "auto", label: "H.264", title: t("Plays everywhere (recommended)") },
    { value: "hevc", label: "HEVC", title: t("Smaller files; needs the HEVC extension or VLC to play") },
    { value: "av1", label: "AV1", title: t("Smallest files; newest GPUs only") },
  ]);
</script>

<main>
  {#if snap?.warnings.length}
    <Banner>{t("Some settings in the file were invalid and were reset: {list}", { list: snap.warnings.join("; ") })}</Banner>
  {/if}
  {#if tray?.status?.mic_connected === false}
    <Banner>{t("Your microphone isn't connected, so the mic track is silent until it's back.")}</Banner>
  {/if}
  {#if tray?.status?.audio_error}
    <Banner>{t("Audio couldn't start, so clips are recorded without sound.")} {tray.status.audio_error}</Banner>
  {/if}

  {#if loadError}
    <Banner tone="error">{t("Couldn't load settings: {error}", { error: loadError })}</Banner>
  {:else if !settings || !snap}
    <div class="skeleton" aria-busy="true" aria-label={t("Loading settings")}>
      {#each Array(6) as _, i (i)}<div class="sk-row"></div>{/each}
    </div>
  {:else}
    <section aria-labelledby="h-replay">
      <h2 id="h-replay">{t("Instant replay")}</h2>
      <Row label={t("Replay length")} hint={t("How much of the past the hotkey saves.")}>
        <Segmented
          label={t("Replay length")}
          value={settings.replay_seconds}
          options={snap.replay_presets.map((s) => ({ value: s, label: replayShort(s), title: replayLabel(s) }))}
          onchange={(v) => change({ replay_seconds: v })}
        />
      </Row>
      <Row label={t("Save hotkey")} hint={t("Works everywhere, including in fullscreen games.")}>
        <HotkeyField
          value={settings.hotkey}
          conflict={hotkeyConflict}
          validate={api.checkHotkey}
          onchange={(v) => change({ hotkey: v })}
        />
      </Row>
      <Row label={t("Save location")} hint={t("Clips go into a folder per game.")}>
        <span class="path" title={saveDir}>{saveDir}</span>
        <Button onclick={pickFolder} disabled={!isDesktop}>{t("Change…")}</Button>
        <Button variant="ghost" onclick={() => api.openFolder(saveDir)} title={t("Open in File Explorer")}>{t("Open")}</Button>
        {#snippet below()}
          {#if settings?.save_dir}
            <div class="below-right">
              <Button variant="ghost" onclick={() => change({ save_dir: "" })}>{t("Use default (Videos\\JondisClip)")}</Button>
            </div>
          {/if}
        {/snippet}
      </Row>
      <Row label={t("Save sound")} hint={t("A short sound confirms the hotkey, even in fullscreen.")} id="t-sound">
        <Toggle id="t-sound" label={t("Save sound")} checked={settings.save_sound} onchange={(v) => change({ save_sound: v })} />
      </Row>
    </section>

    <section aria-labelledby="h-video">
      <h2 id="h-video">{t("Video")}</h2>
      <Row label={t("Quality")} hint={est ? t("Up to {perMin} per minute in busy scenes (a {length} clip up to {perClip}). Calm scenes use much less.", { perMin: est.perMin, length: replayLabel(settings.replay_seconds), perClip: est.perClip }) : undefined}>
        <Segmented label={t("Quality")} value={settings.quality} options={qualities} onchange={(v) => change({ quality: v })} />
      </Row>
      <Row label={t("Resolution")} hint={est ? t("Records at {size}. It never upscales.", { size: est.out }) : undefined}>
        <Segmented label={t("Resolution")} value={settings.resolution} options={resolutions} onchange={(v) => change({ resolution: v })} />
      </Row>
      <Row label={t("Frame rate")} hint={t("Higher is smoother and uses more space.")}>
        <Segmented
          label={t("Frame rate")}
          value={settings.fps}
          options={snap.fps_presets.map((f) => ({ value: f, label: `${f}` }))}
          onchange={(v) => change({ fps: v })}
        />
      </Row>
    </section>

    <section aria-labelledby="h-audio">
      <h2 id="h-audio">{t("Audio")}</h2>
      <Row label={t("Game and system audio")} hint={t("Everything you hear through your speakers or headset.")} id="t-sys">
        <Toggle id="t-sys" label={t("Game and system audio")} checked={settings.system_audio} onchange={(v) => change({ system_audio: v })} />
      </Row>
      <Row label={t("Microphone")} hint={t("Saved as its own track, so it can be removed from a clip later.")} id="t-mic">
        <Toggle id="t-mic" label={t("Microphone")} checked={settings.microphone} onchange={(v) => change({ microphone: v })} />
      </Row>
      {#if settings.microphone}
        <Row label={t("Microphone device")} hint={micHint} id="s-mic">
          <Select
            id="s-mic"
            label={t("Microphone device")}
            value={settings.microphone_device}
            options={micOptions}
            onopen={refreshMics}
            onchange={(v) => change({ microphone_device: v })}
          />
        </Row>
        <Row label={t("Microphone gain")} hint={t("Boosts or lowers only your voice. Applies instantly, without restarting recording.")} id="g-mic">
          <Slider
            id="g-mic"
            label={t("Microphone gain")}
            min={-12}
            max={24}
            value={gainDb}
            format={(v) => (v > 0 ? `+${v} dB` : `${v} dB`)}
            oninput={(v) => (gainPreview = v)}
            onchange={async (v) => {
              await change({ microphone_gain_db: v });
              gainPreview = null;
            }}
          />
        </Row>
        <Row label={t("Level")} hint={levelHint}>
          <LevelMeter peakDb={level.peak} rmsDb={level.rms} hold={level.hold} />
        </Row>
        <Row
          label={t("Low-cut filter")}
          hint={t("Removes rumble below 80 Hz (desk bumps, boom from speaking close, breath pops). Your voice is unaffected. Turn it on if the mic check suggests it.")}
          id="t-lowcut"
        >
          <Toggle
            id="t-lowcut"
            label={t("Low-cut filter")}
            checked={settings.microphone_low_cut}
            onchange={(v) => change({ microphone_low_cut: v })}
          />
        </Row>
        <Row label={t("Mic check")} hint={t("Record 8 seconds of your voice and get advice on the mic's gain knob and settings. Nothing is saved except the measurements.")} stacked>
          <MicCheck
            device={settings.microphone_device}
            gainDb={settings.microphone_gain_db}
            lowCut={settings.microphone_low_cut}
            onUseGain={(db) => change({ microphone_gain_db: db })}
          />
        </Row>
      {/if}
    </section>

    <section aria-labelledby="h-share">
      <h2 id="h-share">{t("Sharing")}</h2>
      <Row
        label={t("Discord upload limit")}
        hint={t("Share to Discord keeps files under this size. 19 MB is safe on a free account; Nitro allows more, which means better quality.")}
      >
        <Segmented
          label={t("Discord upload limit")}
          value={String(settings.discord_limit_mb)}
          options={discordLimits(settings.discord_limit_mb)}
          onchange={(v) => change({ discord_limit_mb: Number(v) })}
        />
      </Row>
    </section>

    <section aria-labelledby="h-drive">
      <h2 id="h-drive">Google Drive</h2>
      <Row
        label={t("Drive folder")}
        hint={settings.drive_dir
          ? t("Save to Drive copies clips into a JondisClip folder here. Google Drive for desktop uploads them from there; JondisClip never uploads anything itself.")
          : driveSuggestion
            ? t("Found Google Drive for desktop at {path}. Use it to turn on Save to Drive.", { path: driveSuggestion })
            : t("Optional. Choose the folder Google Drive for desktop syncs (usually G:\\My Drive) to turn on Save to Drive.")}
      >
        <div class="drive">
          {#if settings.drive_dir}
            <Button variant="ghost" onclick={pickDrive}>{t("Change…")}</Button>
            <Button variant="ghost" onclick={() => setDrive("")}>{t("Turn off")}</Button>
          {:else}
            {#if driveSuggestion}
              <Button onclick={() => setDrive(driveSuggestion ?? "")}>{t("Use {path}", { path: driveSuggestion })}</Button>
            {/if}
            <Button variant="ghost" onclick={pickDrive}>{t("Choose…")}</Button>
          {/if}
        </div>
        {#snippet below()}
          {#if settings?.drive_dir}
            <div class="dcard" class:bad={!driveChecking && driveCheck?.state === "problem"}>
              <span class="dicon" aria-hidden="true">
                {#if driveChecking}<span class="spin"></span>
                {:else if driveCheck?.state === "ok"}<svg viewBox="0 0 16 16"><path d="M3.5 8.5l3 3 6-7" /></svg>
                {:else}<svg viewBox="0 0 16 16"><path d="M8 4.5v4.5M8 11.5v.01" /></svg>{/if}
              </span>
              <div class="dtext" role="status">
                <span class="dpath" title={settings.drive_dir}>{settings.drive_dir}</span>
                <span class="dmsg">
                  {#if driveChecking}{t("Checking the folder…")}
                  {:else if driveCheck?.state === "ok"}{t("Found and writable. Google Drive for desktop uploads what you save here.")}
                  {:else}{driveCheck?.message ?? t("Not checked yet.")}{/if}
                </span>
              </div>
              <Button variant="ghost" disabled={driveChecking} onclick={checkDrive}>{t("Check again")}</Button>
              <Button variant="ghost" disabled={driveCheck?.state !== "ok"} onclick={() => api.driveOpen().catch((e) => showToast(String(e), "error"))}>{t("Open in Explorer")}</Button>
            </div>
          {/if}
        {/snippet}
      </Row>
      {#if settings.drive_dir}
        <Row
          label={t("Keep my folders")}
          hint={t("Clips in a library folder go into a Drive folder with the same name. Off: sorted by game.")}
          id="t-drive-folders"
        >
          <Toggle id="t-drive-folders" label={t("Keep my folders")} checked={settings.drive_keep_folders} onchange={(v) => change({ drive_keep_folders: v })} />
        </Row>
      {/if}
    </section>

    <section aria-labelledby="h-look">
      <h2 id="h-look">{t("Appearance")}</h2>
      <Row label={t("Theme")} hint={t("Accent colour for buttons, selection and highlights. Recording and warning colours don't change.")} stacked>
        <div class="themes" role="radiogroup" aria-label={t("Theme")}>
          {#each THEMES as th (th.id)}
            <label class="theme" class:on={settings.theme === th.id} style:--sw={th.color}>
              <input type="radio" name="theme" value={th.id} checked={settings.theme === th.id} onchange={() => setTheme(th.id)} />
              <span class="sw" aria-hidden="true"><span class="sw-bar"></span><span class="sw-dot"></span></span>
              <span class="tname">{t(th.label)}</span>
            </label>
          {/each}
        </div>
      </Row>
    </section>

    <section aria-labelledby="h-lang">
      <h2 id="h-lang">{t("Language and dates")}</h2>
      <Row label={t("Language")} hint={t("For this window and the tray.")} id="s-lang">
        <Select
          id="s-lang"
          label={t("Language")}
          value={settings.language}
          options={LANGUAGES.map((l) => ({ value: l.id, label: l.label }))}
          onchange={(v) => setLanguage(v, settings?.date_format ?? "ymd")}
        />
      </Row>
      <Row label={t("Date format")} hint={t("Used in the app and in the names of new clips. Clips you already have keep their names.")}>
        <Segmented label={t("Date format")} value={settings.date_format} options={dateFormats} onchange={(v) => setLanguage(settings?.language ?? "en", v)} />
      </Row>
    </section>

    <section aria-labelledby="h-general">
      <h2 id="h-general">{t("General")}</h2>
      <Row label={t("Start with Windows")} hint={t("Start JondisClip in the tray when you sign in.")} id="t-boot">
        <Toggle id="t-boot" label={t("Start with Windows")} checked={settings.start_with_windows} onchange={(v) => change({ start_with_windows: v })} />
      </Row>
      <Row label={t("Check for updates")} hint={t("Asks GitHub for a newer version when JondisClip starts. Nothing is installed without your click.")} id="t-updates">
        <Toggle id="t-updates" label={t("Check for updates")} checked={settings.check_for_updates} onchange={(v) => change({ check_for_updates: v })} />
      </Row>
      {#if tray?.running && tray.status?.update_version}
        <Row label={t("Version {version} is available", { version: tray.status.update_version })} hint={t("You have {current}. Recording continues while it downloads, then JondisClip restarts.", { current: tray.status.version })}>
          <Button onclick={() => api.installUpdate()} disabled={!!tray.status.update_state}>{tray.status.update_state ? t("Downloading…") : t("Install update")}</Button>
        </Row>
      {/if}
    </section>

    <details>
      <summary>{t("Advanced")}</summary>
      <Row label={t("Video codec")} hint={t("H.264 plays everywhere. HEVC and AV1 make smaller files but need extra codecs to play on many PCs.")}>
        <Segmented
          label={t("Video codec")}
          value={settings.codec === "h264" ? "auto" : settings.codec}
          options={codecs}
          onchange={(v) => change({ codec: v })}
        />
      </Row>
    </details>

    <footer>{t("Changes are saved automatically.")}{#if tray?.status?.version} · JondisClip {tray.status.version}{/if}</footer>
  {/if}
</main>

{#if toast}
  <div class="toast {toast.tone}" role={toast.tone === "error" ? "alert" : "status"}>
    {#if toast.tone === "ok"}
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
    {/if}
    {toast.text}
  </div>
{/if}

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: var(--s5) var(--s5) var(--s7);
    display: flex;
    flex-direction: column;
    gap: var(--s5);
  }
  h2 {
    margin: 0 0 var(--s1);
    font-size: var(--fs-xs);
    font-weight: var(--fw-strong);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  section {
    display: flex;
    flex-direction: column;
  }
  .path {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl; /* keep the end of long paths visible */
    text-align: left;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .drive {
    display: flex;
    align-items: center;
    gap: var(--s1);
  }
  .themes {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
    gap: var(--s2);
  }
  .theme {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--s2);
    padding: var(--s2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface);
    cursor: pointer;
    transition:
      border-color var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .theme:hover {
    background: var(--surface-hover);
  }
  .theme.on {
    border-color: var(--sw);
    box-shadow: 0 0 0 1px var(--sw);
  }
  .theme input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .theme:has(input:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  /* A tiny preview: a dark surface with the accent as a button and a dot. */
  .sw {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 36px;
    padding: 0 var(--s2);
    border-radius: var(--radius-sm);
    background: var(--bg);
  }
  .sw-bar {
    width: 44px;
    height: 12px;
    border-radius: var(--radius-pill);
    background: var(--sw);
  }
  .sw-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 2px solid var(--sw);
  }
  .tname {
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
  }
  .theme:not(.on) .tname {
    color: var(--text-muted);
  }
  .dcard {
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: var(--s2) var(--s2) var(--s2) var(--s3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface);
  }
  .dicon {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    flex: none;
    border-radius: 50%;
    background: var(--success-soft);
    color: var(--success);
  }
  .dcard.bad .dicon {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .dicon svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .spin {
    width: 12px;
    height: 12px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .dtext {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .dpath {
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dmsg {
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .dcard.bad .dmsg {
    color: var(--warning);
  }
  .below-right {
    display: flex;
    justify-content: flex-end;
  }
  details {
    border-top: 1px solid var(--border);
    padding-top: var(--s3);
  }
  summary {
    cursor: pointer;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: var(--fw-strong);
    padding: var(--s1) 0;
    width: max-content;
    border-radius: var(--radius-sm);
  }
  summary:hover {
    color: var(--text);
  }
  details :global(.row:first-of-type) {
    border-top: none;
  }
  footer {
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
  .skeleton {
    display: flex;
    flex-direction: column;
    gap: var(--s3);
  }
  .sk-row {
    height: 44px;
    border-radius: var(--radius-md);
    background: linear-gradient(90deg, var(--surface) 0%, var(--surface-raised) 50%, var(--surface) 100%);
    background-size: 200% 100%;
    animation: shimmer 1.2s linear infinite;
  }
  @keyframes shimmer {
    to {
      background-position: -200% 0;
    }
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: var(--s5);
    transform: translateX(-50%);
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    max-width: calc(100vw - 48px);
    padding: var(--s2) var(--s4);
    border-radius: var(--radius-pill);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-sm);
    animation: rise var(--t-med) var(--ease);
  }
  .toast svg {
    width: 14px;
    height: 14px;
    color: var(--success);
  }
  .toast.error {
    color: var(--text);
    box-shadow: var(--shadow-pop), inset 0 0 0 1px var(--danger);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translate(-50%, 6px);
    }
  }
</style>
