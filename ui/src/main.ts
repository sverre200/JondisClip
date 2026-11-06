import { mount } from "svelte";
import "./tokens.css";
import App from "./App.svelte";
import { api } from "./lib/api";
import { applyCachedTheme, applyTheme } from "./lib/theme";
import { applyCachedLocale, setLocale } from "./lib/i18n.svelte";

// Theme and language first (the window is still hidden), then confirm them
// from settings.toml.
applyCachedTheme();
applyCachedLocale();
void api
  .load()
  .then((s) => {
    applyTheme(s.settings.theme);
    setLocale(s.settings.language, s.settings.date_format);
  })
  .catch(() => {});

const target = document.getElementById("app");
if (!target) throw new Error("missing #app");
export default mount(App, { target });
