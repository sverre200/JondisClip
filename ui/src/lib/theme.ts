// Window themes (all dark): each tints the surfaces, borders and text
// towards its hue (palettes from JondisBoard) and sets its accent. The choice is stored in
// settings.toml (`theme`); localStorage keeps a copy so the right colours are
// on <html> before the first paint.
export const THEMES = [
  { id: "crimson", label: "Crimson", color: "#e5484d" },
  { id: "toxic", label: "Toxic Green", color: "#45e33f" },
  { id: "midnight", label: "Midnight Purple", color: "#7c5cff" },
  { id: "ocean", label: "Ocean Blue", color: "#3b8cff" },
  { id: "sunset", label: "Sunset Orange", color: "#ff8a2b" },
  { id: "graphite", label: "Graphite", color: "#e6e6ea" },
] as const;
export type ThemeId = (typeof THEMES)[number]["id"];

const KEY = "jc.theme";
const known = (t: string | null): t is ThemeId => THEMES.some((x) => x.id === t);

export function applyTheme(theme: string | null | undefined) {
  const t: ThemeId = known(theme ?? null) ? (theme as ThemeId) : "crimson";
  if (t === "crimson") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = t;
  try {
    localStorage.setItem(KEY, t);
  } catch {
    // storage unavailable: the setting still applies for this session
  }
}

/** Before mounting: the last theme seen, without waiting for the backend. */
export function applyCachedTheme() {
  let t: string | null = null;
  try {
    t = localStorage.getItem(KEY);
  } catch {
    // ignore
  }
  applyTheme(t);
}
