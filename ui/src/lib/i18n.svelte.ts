// Window language and date format (ADR-0018).
//
// Strings are written in English in the components and wrapped in t(); the
// English text is the key into the Norwegian dictionaries. Placeholders are
// {name}. A string missing from a dictionary shows in English (and
// `npm run check` lists it). Both settings live in settings.toml
// (`language`, `date_format`); localStorage keeps a copy so the first paint
// is already in the right language.
import { nb } from "./i18n/nb";
import { nn } from "./i18n/nn";

export const LANGUAGES = [
  { id: "en", label: "English" },
  { id: "nb", label: "Norsk bokmål" },
  { id: "nn", label: "Norsk nynorsk" },
] as const;
export type Lang = (typeof LANGUAGES)[number]["id"];
export type DateFormat = "ymd" | "dmy" | "mdy";

const DICTS: Record<Lang, Record<string, string> | null> = { en: null, nb, nn };
const INTL: Record<Lang, string> = { en: "en-GB", nb: "nb-NO", nn: "nn-NO" };

export const locale = $state<{ lang: Lang; dateFormat: DateFormat }>({ lang: "en", dateFormat: "ymd" });

const KEY = "jc.locale";

export function setLocale(lang: string | null | undefined, dateFormat: string | null | undefined) {
  const l: Lang = lang === "nb" || lang === "nn" ? lang : "en";
  const f: DateFormat = dateFormat === "dmy" || dateFormat === "mdy" ? dateFormat : "ymd";
  locale.lang = l;
  locale.dateFormat = f;
  document.documentElement.lang = l === "en" ? "en" : l;
  try {
    localStorage.setItem(KEY, `${l}|${f}`);
  } catch {
    // storage unavailable: still applies for this session
  }
}

/** Before mounting: the last language and date format seen. */
export function applyCachedLocale() {
  let v = "";
  try {
    v = localStorage.getItem(KEY) ?? "";
  } catch {
    // ignore
  }
  const [l, f] = v.split("|");
  setLocale(l, f);
}

type Vars = Record<string, string | number>;

function fill(s: string, vars?: Vars): string {
  if (!vars) return s;
  return s.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m));
}

/** Translates `en` (the English text is the key) and fills {placeholders}. */
export function t(en: string, vars?: Vars): string {
  const d = DICTS[locale.lang];
  return fill(d?.[en] ?? en, vars);
}

/** `one` when n is 1, else `other`; both translated, with {n} filled in. */
export function tn(n: number, one: string, other: string, vars?: Vars): string {
  return t(n === 1 ? one : other, { n, ...vars });
}

/** BCP 47 tag for Intl (dates, sorting). */
export function intlLocale(): string {
  return INTL[locale.lang];
}

// ---------------------------------------------------------------- dates

const pad = (n: number) => String(n).padStart(2, "0");

/** A full date in the chosen format: 2026-11-06, 06.11.2026 or 11/06/2026. */
export function numericDate(d: Date): string {
  const y = d.getFullYear();
  const m = pad(d.getMonth() + 1);
  const day = pad(d.getDate());
  if (locale.dateFormat === "dmy") return `${day}.${m}.${y}`;
  if (locale.dateFormat === "mdy") return `${m}/${day}/${y}`;
  return `${y}-${m}-${day}`;
}

/** A date without its year when it's this year: 06.11., 11/06 or 2026-11-06 (ISO keeps the year). */
export function shortNumericDate(d: Date, now = new Date()): string {
  if (d.getFullYear() !== now.getFullYear() || locale.dateFormat === "ymd") return numericDate(d);
  const m = pad(d.getMonth() + 1);
  const day = pad(d.getDate());
  return locale.dateFormat === "dmy" ? `${day}.${m}.` : `${m}/${day}`;
}

/** "Wednesday", "onsdag" … */
export function weekday(d: Date): string {
  const w = d.toLocaleDateString(intlLocale(), { weekday: "long" });
  return locale.lang === "en" ? w : w.charAt(0).toUpperCase() + w.slice(1);
}
