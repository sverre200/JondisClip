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
  import { LANGUAGES, intlLocale, setLocale, t } from "./lib/i18n.svelte";

  // Tray status is polled by the shell (App.svelte) for both pages; `onpoll`
  // asks for it right now (while an update check or download is under way).
  let { tray, onpoll }: { tray: TrayView | null; onpoll: () => Promise<void> } = $props();

  // One page per group, chosen in the side menu (the last one is remembered).
  type Tab = "recording" | "audio" | "sharing" | "appearance" | "general" | "updates";
  const TAB_IDS: Tab[] = ["recording", "audio", "sharing", "appearance", "general", "updates"];
  const savedTab = localStorage.getItem("jc.settings.tab") as Tab | null;
  let tab = $state<Tab>(savedTab && TAB_IDS.includes(savedTab) ? savedTab : "recording");
  $effect(() => localStorage.setItem("jc.settings.tab", tab));
  const tabs: { id: Tab; label: string; icon: string }[] = $derived([
    { id: "recording", label: t("Recording"), icon: "M8 2.5a5.5 5.5 0 1 0 0 11a5.5 5.5 0 0 0 0-11zM8 6a2 2 0 1 0 0 4a2 2 0 0 0 0-4z" },
    { id: "audio", label: t("Audio"), icon: "M8 2a2 2 0 0 0-2 2v4a2 2 0 0 0 4 0V4a2 2 0 0 0-2-2zM3.5 8a4.5 4.5 0 0 0 9 0M8 12.5v2" },
    { id: "sharing", label: t("Sharing"), icon: "M8 10V2.5M5 5l3-3 3 3M3 9.5v3a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1v-3" },
    { id: "appearance", label: t("Appearance"), icon: "M8 2.5c2.5 3 4 4.9 4 6.9a4 4 0 0 1-8 0c0-2 1.5-3.9 4-6.9z" },
    { id: "general", label: t("General"), icon: "M2.5 5h6M11.5 5h2M2.5 11h2M7.5 11h6M10 3.5v3M6 9.5v3" },
    { id: "updates", label: t("Updates"), icon: "M8 2.5v7M5 7l3 3 3-3M3 12.5h10" },
  ]);
  let pane = $state<HTMLElement>();
  function go(id: Tab) {
    tab = id;
    pane?.scrollTo({ top: 0 });
  }
  function navKey(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const els = [...((e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    const i = els.indexOf(e.currentTarget as HTMLButtonElement);
    const next = els[(i + (e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : -1) + els.length) % els.length];
    next?.focus();
    next?.click();
  }

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
    if (!micOn || tab !== "audio") return;
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
  // ---- updates: the card reacts at once, then follows the tray's status ----
  // When "Check now" was clicked (0: no check started from here).
  let checkAt = $state(0);
  let sawChecking = false;
  let checkedAt = $state<Date | null>(null);
  let checkTimedOut = $state(false);
  // "Install update" was clicked and the tray hasn't reported the download yet.
  let installAt = $state(0);

  async function checkNow() {
    if (checkAt) return;
    checkTimedOut = false;
    checkedAt = null;
    sawChecking = false;
    checkAt = Date.now();
    try {
      await api.checkUpdate();
    } catch (e) {
      checkAt = 0;
      showToast(String(e), "error");
    }
  }
  async function install() {
    if (installAt) return;
    installAt = Date.now();
    try {
      await api.installUpdate();
    } catch (e) {
      installAt = 0;
      showToast(String(e), "error");
    }
  }
  // The shell polls every 1.5 s; while something is happening, ask 4× a second.
  const busy = $derived(!!checkAt || !!installAt || !!tray?.status?.update_state);
  $effect(() => {
    if (!busy) return;
    const timer = setInterval(async () => {
      await onpoll();
      const s = tray?.status;
      if (checkAt) {
        const age = Date.now() - checkAt;
        if (s?.update_check === "checking") sawChecking = true;
        const settled = !!s?.update_version || s?.update_check === "current" || s?.update_check === "failed";
        // Long enough to be seen, and not the answer from an earlier check.
        if (settled && age >= 700 && (sawChecking || age >= 2000)) {
          checkAt = 0;
          checkedAt = new Date();
        } else if (age > 30_000) {
          checkAt = 0;
          checkTimedOut = true;
        }
      }
      if (installAt && (s?.update_state || Date.now() - installAt > 5000)) installAt = 0;
    }, 250);
    return () => clearInterval(timer);
  });

  type UpdateKind = "off" | "checking" | "available" | "downloading" | "installing" | "current" | "failed" | "idle";
  const up: { kind: UpdateKind; title: string; text: string; percent?: number | null } = $derived.by(() => {
    const s = tray?.status;
    if (!tray?.running || !s) return { kind: "off", title: "JondisClip", text: t("Start JondisClip to check for updates.") };
    const mine = t("JondisClip {version}", { version: s.version });
    if (s.update_state === "installing")
      return { kind: "installing", title: t("Installing {version}…", { version: s.update_version ?? "" }), text: t("JondisClip restarts by itself in a moment.") };
    if (s.update_version && (s.update_state === "downloading" || installAt)) {
      const percent = s.update_progress ?? null;
      return {
        kind: "downloading",
        title: t("Downloading {version}…", { version: s.update_version }),
        text: percent === null ? t("Recording continues meanwhile.") : t("{percent} % downloaded. Recording continues meanwhile.", { percent }),
        percent,
      };
    }
    if (checkAt || s.update_check === "checking") return { kind: "checking", title: t("Checking for updates…"), text: t("Asking GitHub for the newest version.") };
    if (s.update_version)
      return {
        kind: "available",
        title: t("Version {version} is available", { version: s.update_version }),
        text: t("You have {current}. Recording continues while it downloads, then JondisClip restarts.", { current: s.version }),
      };
    if (checkTimedOut || s.update_check === "failed")
      return { kind: "failed", title: t("Couldn't check for updates"), text: t("Couldn't reach GitHub. Check your internet connection and try again.") };
    if (s.update_check === "current")
      return {
        kind: "current",
        title: t("You have the latest version"),
        text: checkedAt
          ? t("{version} · checked at {time}", { version: mine, time: checkedAt.toLocaleTimeString(intlLocale(), { hour: "2-digit", minute: "2-digit" }) })
          : mine,
      };
    return { kind: "idle", title: mine, text: t("See whether a newer version is out.") };
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

<div class="settings">
<nav class="snav" aria-label={t("Settings sections")}>
  {#each tabs as x (x.id)}
    <button type="button" class="srow" class:on={tab === x.id} aria-current={tab === x.id ? "page" : undefined} onclick={() => go(x.id)} onkeydown={navKey}>
      <svg class="glyph" viewBox="0 0 16 16" aria-hidden="true"><path d={x.icon} fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
      <span class="name">{x.label}</span>
      {#if x.id === "updates" && tray?.running && tray.status?.update_version}<span class="dot" title={t("Version {version} is available", { version: tray.status.update_version })}></span>{/if}
    </button>
  {/each}
</nav>
<main bind:this={pane}>
  <div class="page">
  <h1>{tabs.find((x) => x.id === tab)?.label}</h1>
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
    {#if tab === "recording"}
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
      <Row label={t("Video codec")} hint={t("H.264 plays everywhere. HEVC and AV1 make smaller files but need extra codecs to play on many PCs.")}>
        <Segmented
          label={t("Video codec")}
          value={settings.codec === "h264" ? "auto" : settings.codec}
          options={codecs}
          onchange={(v) => change({ codec: v })}
        />
      </Row>
    </section>
    {/if}

    {#if tab === "audio"}
    <section aria-label={t("Audio")}>
      <Row label={t("Game and system audio")} hint={t("Everything you hear through your speakers or headset.")} id="t-sys">
        <Toggle id="t-sys" label={t("Game and system audio")} checked={settings.system_audio} onchange={(v) => change({ system_audio: v })} />
      </Row>
      <Row label={t("Microphone")} hint={t("Saved as its own track, so it can be removed from a clip later.")} id="t-mic">
        <Toggle id="t-mic" label={t("Microphone")} checked={settings.microphone} onchange={(v) => change({ microphone: v })} />
      </Row>
      {#if settings.microphone}
        <Row label={t("Microphone device")} hint={micHint} hintLines={2} id="s-mic">
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
        <!-- The hint follows your voice: room for two lines, so the page doesn't jump. -->
        <Row label={t("Level")} hint={levelHint} hintLines={2}>
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
    {/if}

    {#if tab === "sharing"}
    <section aria-labelledby="h-share">
      <h2 id="h-share">Discord</h2>
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
    {/if}

    {#if tab === "appearance"}
    <section aria-label={t("Appearance")}>
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
    {/if}

    {#if tab === "general"}
    <section aria-labelledby="h-start">
      <h2 id="h-start">{t("Startup")}</h2>
      <Row label={t("Start with Windows")} hint={t("Start JondisClip in the tray when you sign in.")} id="t-boot">
        <Toggle id="t-boot" label={t("Start with Windows")} checked={settings.start_with_windows} onchange={(v) => change({ start_with_windows: v })} />
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

    {/if}

    {#if tab === "updates"}
    <section aria-label={t("Updates")}>
      <div class="dcard ucard {up.kind}">
        <span class="dicon" aria-hidden="true">
          {#if up.kind === "checking" || up.kind === "downloading" || up.kind === "installing"}<span class="spin"></span>
          {:else if up.kind === "current"}<svg viewBox="0 0 16 16"><path d="M3.5 8.5l3 3 6-7" /></svg>
          {:else if up.kind === "available"}<svg viewBox="0 0 16 16"><path d="M8 3v7M5 7.5l3 3 3-3M4 13h8" /></svg>
          {:else if up.kind === "failed"}<svg viewBox="0 0 16 16"><path d="M8 4.5v4.5M8 11.5v.01" /></svg>
          {:else}<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="2.5" /></svg>{/if}
        </span>
        <div class="dtext" role="status" aria-live="polite">
          <span class="dpath">{up.title}</span>
          <span class="dmsg">{up.text}</span>
          {#if up.kind === "downloading" || up.kind === "installing"}
            <div
              class="bar"
              class:unknown={up.kind === "installing" || up.percent == null}
              role="progressbar"
              aria-label={up.title}
              aria-valuemin="0"
              aria-valuemax="100"
              aria-valuenow={up.kind === "downloading" ? (up.percent ?? undefined) : undefined}
            >
              <span style:width="{up.kind === "installing" ? 100 : (up.percent ?? 30)}%"></span>
            </div>
          {/if}
        </div>
        {#if up.kind === "available"}
          <Button variant="primary" onclick={install}>{t("Install update")}</Button>
        {:else if up.kind === "failed"}
          <Button onclick={checkNow}>{t("Try again")}</Button>
        {:else if up.kind !== "downloading" && up.kind !== "installing"}
          <Button onclick={checkNow} disabled={up.kind === "off" || up.kind === "checking"}>{up.kind === "checking" ? t("Checking…") : t("Check now")}</Button>
        {/if}
      </div>
      <Row label={t("Check for updates")} hint={t("Asks GitHub for a newer version when JondisClip starts. Nothing is installed without your click.")} id="t-updates">
        <Toggle id="t-updates" label={t("Check for updates")} checked={settings.check_for_updates} onchange={(v) => change({ check_for_updates: v })} />
      </Row>
    </section>
    {/if}

    <footer>{t("Changes are saved automatically.")}</footer>
  {/if}
  </div>
</main>
</div>

{#if toast}
  <div class="toast {toast.tone}" role={toast.tone === "error" ? "alert" : "status"}>
    {#if toast.tone === "ok"}
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
    {/if}
    {toast.text}
  </div>
{/if}

<style>
  .settings {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  /* Same look as the library's folder menu. */
  .snav {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 208px;
    flex: none;
    padding: var(--s3) var(--s2);
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .srow {
    display: flex;
    align-items: center;
    gap: var(--s2);
    width: 100%;
    height: 32px;
    padding: 0 var(--s2);
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
    flex: none;
  }
  .srow:hover {
    background: var(--surface-hover);
    color: var(--text);
  }
  .srow.on {
    background: var(--surface-raised);
    color: var(--text);
    font-weight: var(--fw-strong);
  }
  .glyph {
    width: 16px;
    height: 16px;
    flex: none;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    width: 8px;
    height: 8px;
    flex: none;
    border-radius: 50%;
    background: var(--accent);
  }
  main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .page {
    max-width: 720px;
    margin: 0 auto;
    padding: var(--s5) var(--s5) var(--s7);
    display: flex;
    flex-direction: column;
    gap: var(--s5);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--fs-xl);
    font-weight: var(--fw-strong);
  }
  /* Narrow windows: the menu becomes a strip above the page. */
  @media (max-width: 760px) {
    .settings {
      flex-direction: column;
    }
    .snav {
      flex-direction: row;
      gap: var(--s1);
      width: auto;
      padding: var(--s2) var(--s3);
      border-right: 0;
      border-bottom: 1px solid var(--border);
      overflow: auto hidden;
    }
    .srow {
      width: auto;
      padding: 0 var(--s3);
    }
    .name {
      flex: none;
      overflow: visible;
    }
  }
  .ucard {
    padding: var(--s3);
    margin-bottom: var(--s2);
  }
  .ucard .dicon {
    width: 32px;
    height: 32px;
    background: var(--surface-raised);
    color: var(--text-muted);
  }
  .ucard .dicon svg {
    width: 16px;
    height: 16px;
  }
  .ucard.current .dicon {
    background: var(--success-soft);
    color: var(--success);
  }
  .ucard.available,
  .ucard.downloading,
  .ucard.installing {
    border-color: var(--accent);
  }
  .ucard.available .dicon,
  .ucard.checking .dicon,
  .ucard.downloading .dicon,
  .ucard.installing .dicon {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .ucard.failed .dicon {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .ucard .dpath {
    white-space: normal;
  }
  .bar {
    height: 4px;
    margin-top: var(--s2);
    border-radius: var(--radius-pill);
    background: var(--surface-raised);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
    transition: width var(--t-med) linear;
  }
  .bar.unknown span {
    animation: slide 1.2s var(--ease) infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar.unknown span,
    .spin {
      animation-duration: 4s;
    }
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
