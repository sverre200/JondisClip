import { mount } from "svelte";
import "./tokens.css";
import App from "./App.svelte";
import { api } from "./lib/api";
import { applyCachedTheme, applyTheme } from "./lib/theme";

// Theme first (the window is still hidden), then confirm it from settings.toml.
applyCachedTheme();
void api
  .load()
  .then((s) => applyTheme(s.settings.theme))
  .catch(() => {});

const target = document.getElementById("app");
if (!target) throw new Error("missing #app");
export default mount(App, { target });
