<script lang="ts">
  import { onMount } from "svelte";
  import { api, isDesktop, type InputDevice, type Settings, type Snapshot, type TrayView } from "./lib/api";
  import { replayLabel } from "./lib/format";
  import Button from "./lib/Button.svelte";
  import HotkeyField from "./lib/HotkeyField.svelte";
  import Keycaps from "./lib/Keycaps.svelte";
  import Row from "./lib/Row.svelte";
  import Select from "./lib/Select.svelte";
  import Toggle from "./lib/Toggle.svelte";
  import { LANGUAGES, setLocale, t } from "./lib/i18n.svelte";

  // First-run setup (ADR-0022): the few choices a new user should see once.
  // Every row saves at once, like Settings; "Start clipping" only marks the
  // setup as done so this screen doesn't come back.
  let { tray, ondone }: { tray: TrayView | null; ondone: () => void } = $props();

  let snap = $state<Snapshot | null>(null);
  let settings = $state<Settings | null>(null);
  let mics = $state<InputDevice[] | null>(null);
  let error = $state<string | null>(null);
  let card = $state<HTMLDivElement>();

  onMount(async () => {
    try {
      snap = await api.load();
      settings = snap.settings;
      mics = await api.listMicrophones().catch(() => null);
    } catch (e) {
      error = String(e);
    }
  });

  async function change(patch: Partial<Settings>) {
    if (!settings) return;
    const before = settings;
    settings = { ...settings, ...patch }; // optimistic
    try {
      settings = await api.update(patch);
      error = null;
    } catch (e) {
      settings = before;
      error = String(e);
    }
  }

  async function setLanguage(lang: string) {
    if (!settings) return;
    const before = settings.language;
    setLocale(lang, settings.date_format);
    await change({ language: lang });
    if (settings.language !== lang) setLocale(before, settings.date_format);
  }

  async function pickFolder() {
    if (!settings || !snap) return;
    const picked = await api.pickFolder(settings.save_dir || snap.default_save_dir);
    if (picked) await change({ save_dir: picked });
  }

  async function finish() {
    await change({ setup_done: true });
    if (settings?.setup_done) ondone();
  }

  const saveDir = $derived(settings && snap ? settings.save_dir || snap.default_save_dir : "");
  const micOptions = $derived.by(() => {
    const list = mics ?? [];
    const def = list.find((m) => m.is_default);
    const opts = [{ value: "", label: def ? t("Windows default · {name}", { name: def.name }) : t("Windows default") }];
    for (const m of list) opts.push({ value: m.id, label: m.name });
    return opts;
  });
  const hotkeyConflict = $derived(
    !!tray?.running && !!tray.status && !tray.status.hotkey_ok && tray.status.hotkey === settings?.hotkey,
  );

  function key(e: KeyboardEvent) {
    if (e.key !== "Tab" || !card) return;
    // Keep focus in the card.
    const f = [...card.querySelectorAll<HTMLElement>("button:not(:disabled)")];
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
</script>

<div class="scrim">
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="card" role="dialog" aria-modal="true" aria-labelledby="w-title" bind:this={card} tabindex="-1" onkeydown={key}>
    <h1 id="w-title">{t("Welcome to JondisClip")}</h1>
    {#if settings && snap}
      <p class="lead">
        {t("JondisClip keeps the last {length} of your screen and sound while you play. Press the hotkey to save them as a clip.", { length: replayLabel(settings.replay_seconds) })}
      </p>
      <div class="how">
        <Keycaps hotkey={settings.hotkey} />
        <span>{t("saves a clip. JondisClip stays in the system tray, by the clock.")}</span>
      </div>

      <div class="rows">
        <Row label={t("Language")} id="w-lang">
          <Select id="w-lang" label={t("Language")} value={settings.language} options={LANGUAGES.map((l) => ({ value: l.id, label: l.label }))} onchange={setLanguage} />
        </Row>
        <Row label={t("Save hotkey")} hint={t("Works everywhere, including in fullscreen games.")}>
          <HotkeyField value={settings.hotkey} conflict={hotkeyConflict} validate={api.checkHotkey} onchange={(v) => change({ hotkey: v })} />
        </Row>
        <Row label={t("Save location")} hint={t("Clips go into a folder per game.")}>
          <span class="path" title={saveDir}>{saveDir}</span>
          <Button onclick={pickFolder} disabled={!isDesktop}>{t("Change…")}</Button>
        </Row>
        <Row label={t("Record my microphone")} hint={t("Off: clips have game sound only. On: your voice is saved as its own track.")} id="w-mic">
          <Toggle id="w-mic" label={t("Record my microphone")} checked={settings.microphone} onchange={(v) => change({ microphone: v })} />
        </Row>
        {#if settings.microphone}
          <Row label={t("Microphone device")} id="w-micdev">
            <Select id="w-micdev" label={t("Microphone device")} value={settings.microphone_device} options={micOptions} onchange={(v) => change({ microphone_device: v })} />
          </Row>
        {/if}
        <Row label={t("Start with Windows")} hint={t("Start JondisClip in the tray when you sign in.")} id="w-boot">
          <Toggle id="w-boot" label={t("Start with Windows")} checked={settings.start_with_windows} onchange={(v) => change({ start_with_windows: v })} />
        </Row>
      </div>

      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="foot">
        <span class="later">{t("You can change all of this later in Settings.")}</span>
        <Button variant="primary" onclick={finish}>{t("Start clipping")}</Button>
      </div>
    {:else if error}
      <p class="err" role="alert">{error}</p>
      <div class="foot">
        <span></span>
        <Button onclick={ondone}>{t("Close")}</Button>
      </div>
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: var(--s5);
    background: rgb(0 0 0 / 0.6);
    overflow-y: auto;
  }
  .card {
    width: min(620px, 100%);
    padding: var(--s6);
    border-radius: var(--radius-lg);
    background: var(--surface-raised);
    box-shadow: var(--shadow-pop);
    outline: none;
  }
  h1 {
    margin: 0 0 var(--s2);
    font-family: var(--font-display);
    font-size: var(--fs-xl);
    font-weight: var(--fw-strong);
  }
  .lead {
    margin: 0 0 var(--s3);
    color: var(--text-muted);
  }
  .how {
    display: flex;
    align-items: center;
    gap: var(--s2);
    margin-bottom: var(--s4);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .rows {
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .path {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .err {
    margin: var(--s3) 0 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s4);
    margin-top: var(--s4);
  }
  .later {
    color: var(--text-faint);
    font-size: var(--fs-sm);
  }
</style>
