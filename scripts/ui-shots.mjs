// Screenshots of the panel's pages, for UI reviews (docs/ui-redesign.md):
// each page at each width, theme and language, signed in as a test user of a
// running panel. Needs Google Chrome and `npm ci` in resources/admin.
//
//   RDP_SHOT_PASSWORD=… node scripts/ui-shots.mjs [out-dir]
//
// Environment (all optional but the password):
//   RDP_SHOT_BASE      the admin URL (http://localhost:8000/admin)
//   RDP_SHOT_EMAIL     the user (shot@example.com), a super admin with no
//                      saved language (a saved one wins over RDP_SHOT_LOCALES)
//   RDP_SHOT_PAGES     names to take, comma-separated (all)
//   RDP_SHOT_WIDTHS    1440,768,390
//   RDP_SHOT_THEMES    light,dark
//   RDP_SHOT_LOCALES   en,vi
//   RDP_SHOT_SCALE     the device pixel ratio (1; 2 for README images)
//   RDP_SHOT_FULL      0: the first screen only, as README images are (default: whole pages)
// The host's name is replaced by `server-01` on every shot.
import { createRequire } from "node:module";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const require = createRequire(new URL("../resources/admin/package.json", import.meta.url));
const { chromium } = require("playwright-core");

const env = (name, fallback) => process.env[name]?.trim() || fallback;
const list = (name, fallback) => env(name, fallback).split(",").map((s) => s.trim()).filter(Boolean);
const base = env("RDP_SHOT_BASE", "http://localhost:8000/admin");
const email = env("RDP_SHOT_EMAIL", "shot@example.com");
const password = process.env.RDP_SHOT_PASSWORD;
if (!password) {
  console.error("RDP_SHOT_PASSWORD is required");
  process.exit(2);
}
const out = process.argv[2] ?? "storage/ui-shots";
const widths = list("RDP_SHOT_WIDTHS", "1440,768,390").map(Number);
const themes = list("RDP_SHOT_THEMES", "light,dark");
const locales = list("RDP_SHOT_LOCALES", "en,vi");
const scale = Number(env("RDP_SHOT_SCALE", "1"));
const full = env("RDP_SHOT_FULL", "1") !== "0";

// name, path, and what to do on the page before the shot (a dialog, a menu).
const PAGES = [
  ["dashboard", "/p/dashboard"],
  ["project", "/p/dashboard/demo"],
  ["service-overview", "/p/dashboard/demo/web/overview"],
  ["service-deployments", "/p/dashboard/demo/web/deployments"],
  ["service-environment", "/p/dashboard/demo/web/environment"],
  ["service-domains", "/p/dashboard/demo/web/domains"],
  ["service-redirects", "/p/dashboard/demo/web/redirects"],
  ["service-security", "/p/dashboard/demo/web/security"],
  ["service-maintenance", "/p/dashboard/demo/web/maintenance"],
  ["service-scripts", "/p/dashboard/demo/web/scripts"],
  ["service-source", "/p/dashboard/demo/web/source"],
  ["service-storage", "/p/dashboard/demo/web/storage"],
  ["service-advanced", "/p/dashboard/demo/web/advanced"],
  ["database-overview", "/p/dashboard/demo/db/overview"],
  ["database-credentials", "/p/dashboard/demo/db/credentials"],
  ["database-backups", "/p/dashboard/demo/db/backups"],
  ["domains", "/p/domains"],
  ["actions", "/p/actions"],
  ["settings-panel", "/p/settings/panel"],
  ["settings-version", "/p/settings/version"],
  ["settings-traefik", "/p/settings/traefik"],
  ["settings-certificates", "/p/settings/certificates"],
  ["settings-tunnel", "/p/settings/tunnel"],
  ["settings-branding", "/p/settings/branding"],
  ["settings-git", "/p/settings/git"],
  ["settings-backups", "/p/settings/backups"],
  ["settings-notifications", "/p/settings/notifications"],
  ["settings-tokens", "/p/settings/tokens"],
  ["containers", "/p/containers"],
  ["images", "/p/images"],
  ["volumes", "/p/volumes"],
  ["networks", "/p/networks"],
  ["container-logs", "/p/containers/demo_web/logs"],
  ["service-record", "/r/services/records/9"],
  ["users", "/system/users"],
  ["roles", "/system/roles"],
  ["new-service", "/p/dashboard", (page) => page.getByRole("button", { name: /Add service|\+ Service|Thêm service/ }).first().click()],
  ["add-domain", "/p/dashboard/demo/web/domains", (page) => page.getByRole("button", { name: /Add domain|Thêm domain/ }).first().click()],
  ["templates", "/p/dashboard", async (page) => {
    await page.getByRole("button", { name: /^Templates$|^Template$/ }).last().click();
    await page.waitForTimeout(6000);
  }],
  ["service-logs", "/p/dashboard/demo/web/overview", async (page) => {
    await page.getByRole("button", { name: /^Logs$|^Log$/ }).first().click();
    await page.waitForTimeout(3000);
  }],
  ["service-terminal", "/p/dashboard/demo/web/overview", async (page) => {
    await page.getByRole("button", { name: /^Shell$/ }).first().click();
    await page.waitForTimeout(3000);
    await page.keyboard.type("nginx -v; ls /usr/share/nginx/html; uptime\n");
    await page.waitForTimeout(2000);
  }],
];
const wanted = list("RDP_SHOT_PAGES", PAGES.map(([name]) => name).join(","));

const host = os.hostname();
async function mask(page) {
  await page.evaluate((h) => {
    const short = h.replace(/\.local$/, "");
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n; (n = walker.nextNode()); ) {
      if (n.nodeValue.includes(short)) n.nodeValue = n.nodeValue.split(h).join("server-01").split(short).join("server-01");
    }
  }, host);
}

fs.mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ channel: "chrome" });
let failed = 0;
for (const locale of locales) {
  for (const theme of themes) {
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: scale, colorScheme: theme, locale });
    // The shell's own choices (theme, language), before any page loads.
    await context.addInitScript(([scheme, lang]) => {
      try {
        localStorage.setItem("rustcms.colorScheme", scheme);
        localStorage.setItem("rustcms-admin-locale", lang);
      } catch {}
    }, [theme, locale]);
    const page = await context.newPage();
    await page.goto(`${base}/login`);
    await page.locator("input[type=email]").fill(email);
    await page.locator("input[type=password]").fill(password);
    await page.locator("button[type=submit]").click();
    await page.waitForURL(/\/p\//, { timeout: 20000 });
    for (const width of widths) {
      await page.setViewportSize({ width, height: 900 });
      for (const [name, at, act] of PAGES) {
        if (!wanted.includes(name)) continue;
        const file = path.join(out, `${name}-${width}-${theme}-${locale}.png`);
        try {
          const response = await page.goto(base + at);
          if (response && response.status() >= 400) throw new Error(`HTTP ${response.status()}`);
          await page.waitForLoadState("networkidle", { timeout: 8000 }).catch(() => {});
          await page.waitForTimeout(1200);
          if (act) {
            await act(page);
            await page.waitForTimeout(800);
          }
          // Signed out meanwhile (or sent elsewhere): not the page asked for.
          if (!page.url().includes(at.split("/").slice(0, 3).join("/"))) throw new Error(`landed on ${page.url()}`);
          const lang = await page.evaluate(() => document.documentElement.lang);
          if (lang && !lang.startsWith(locale)) throw new Error(`shown in ${lang}, not ${locale} (the user's saved language wins)`);
          await mask(page);
          await page.screenshot({ path: file, fullPage: full && !act });
          console.log("shot", path.basename(file));
        } catch (err) {
          failed += 1;
          console.log("FAILED", path.basename(file), String(err.message).split("\n")[0]);
        }
      }
    }
    await context.close();
  }
}
await browser.close();
process.exit(failed ? 1 : 0);
