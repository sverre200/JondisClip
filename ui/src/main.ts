import { mount } from "svelte";
import "./tokens.css";
import App from "./App.svelte";

const target = document.getElementById("app");
if (!target) throw new Error("missing #app");
export default mount(App, { target });
