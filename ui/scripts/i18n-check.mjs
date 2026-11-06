// Lists every string passed to t()/tn() in ui/src and checks that the
// Norwegian dictionaries (src/lib/i18n/nb.ts, nn.ts) translate all of them,
// keep every {placeholder}, and have no leftover keys. Part of `npm run check`.
// `node scripts/i18n-check.mjs --list` prints the keys instead.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "src");

function files(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return n === "i18n" ? [] : files(p);
    return /\.(svelte|ts)$/.test(n) ? [p] : [];
  });
}

// A double-quoted JS string literal (no template literals: keys must be static).
const STR = String.raw`"((?:[^"\\]|\\.)*)"`;
const T = new RegExp(String.raw`\bt\(\s*` + STR, "g");
const TN = new RegExp(String.raw`\btn\(\s*[^,]+?,\s*` + STR + String.raw`\s*,\s*` + STR, "g");
const unescape = (s) => JSON.parse(`"${s}"`);

const keys = new Map();
for (const f of files(root)) {
  const src = readFileSync(f, "utf8");
  for (const m of src.matchAll(T)) keys.set(unescape(m[1]), f);
  for (const m of src.matchAll(TN)) {
    keys.set(unescape(m[1]), f);
    keys.set(unescape(m[2]), f);
  }
}
// Labels translated at the call site with t(variable).
for (const k of ["Crimson", "Toxic Green", "Midnight Purple", "Ocean Blue", "Sunset Orange", "Graphite", "Red", "Orange", "Yellow", "Green", "Blue", "Purple", "Gray"]) keys.set(k, "labels");

if (process.argv.includes("--list")) {
  for (const k of [...keys.keys()].sort()) console.log(JSON.stringify(k));
  process.exit(0);
}

function dict(name) {
  const src = readFileSync(join(root, "lib", "i18n", `${name}.ts`), "utf8");
  const body = src.slice(src.indexOf("= {") + 2, src.lastIndexOf("}") + 1);
  return JSON.parse(body);
}

const holes = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");
let bad = 0;
for (const lang of ["nb", "nn"]) {
  const d = dict(lang);
  for (const k of keys.keys()) {
    if (!(k in d)) {
      console.error(`${lang}: missing ${JSON.stringify(k)}`);
      bad++;
    } else if (holes(k) !== holes(d[k])) {
      console.error(`${lang}: placeholders differ for ${JSON.stringify(k)}`);
      bad++;
    }
  }
  for (const k of Object.keys(d)) {
    if (!keys.has(k)) {
      console.error(`${lang}: unused ${JSON.stringify(k)}`);
      bad++;
    }
  }
}
if (bad) {
  console.error(`i18n: ${bad} problem(s)`);
  process.exit(1);
}
console.log(`i18n: ${keys.size} strings, bokmål and nynorsk complete`);
