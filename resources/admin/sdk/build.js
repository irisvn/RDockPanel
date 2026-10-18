#!/usr/bin/env node
// `rustcms-build`: builds every `src/<vendor>/<name>.tsx` of an app's
// `resources/admin` into `js/<vendor>/<name>.js` and `css/<vendor>/<name>.css`
// (one Vite build each: IIFE allows one input). An entry without styles gets
// an empty CSS file, so `include_str!` always finds one; outputs of removed
// entries are deleted (those listed in `.rustcms-build.json`).
import fs from "node:fs";
import path from "node:path";
import { build } from "vite";
import { rustcmsExtension } from "./vite.js";

const root = process.cwd();
const src = path.join(root, "src");
const entries = fs.existsSync(src)
  ? fs
      .readdirSync(src, { withFileTypes: true })
      .filter((d) => d.isDirectory() && /^[a-z0-9-]+$/.test(d.name))
      .flatMap((d) =>
        fs
          .readdirSync(path.join(src, d.name))
          .filter((f) => /^[a-z0-9-]+\.tsx$/.test(f))
          .map((f) => path.join(src, d.name, f)),
      )
      .sort()
  : [];
const wanted = new Set();
for (const entry of entries) {
  const rel = path.relative(src, entry).replace(/\\/g, "/").replace(/\.tsx$/, "");
  const css = path.join(root, "css", `${rel}.css`);
  // A previous build's styles never outlive the entry's last CSS import.
  fs.rmSync(css, { force: true });
  await build(rustcmsExtension({ entry, root }));
  if (!fs.existsSync(css)) {
    fs.mkdirSync(path.dirname(css), { recursive: true });
    fs.writeFileSync(css, "");
  }
  wanted.add(`js/${rel}.js`);
  wanted.add(`css/${rel}.css`);
  console.log(`built ${rel}`);
}
// Outputs of entries that are gone: only files this tool wrote before (its
// manifest), never hand-written scripts beside them.
const manifest = path.join(root, ".rustcms-build.json");
const before = fs.existsSync(manifest) ? JSON.parse(fs.readFileSync(manifest, "utf8")) : [];
for (const rel of before) {
  if (!wanted.has(rel) && fs.existsSync(path.join(root, rel))) {
    fs.rmSync(path.join(root, rel));
    console.log(`removed ${rel}`);
  }
}
fs.writeFileSync(manifest, `${JSON.stringify([...wanted].sort(), null, 2)}\n`);
