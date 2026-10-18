# Vendored scripts

Loaded by `src/providers/admin_panel_provider.rs` (`extension_script`), in this order:
`csp-shim.js`, `xterm.js`, `addon-fit.js`.

| File | Source | License |
|---|---|---|
| `xterm.js` | `@xterm/xterm` 6.0.0, `lib/xterm.js` (UMD; source map line removed) | MIT (`LICENSE-xterm`) |
| `addon-fit.js` | `@xterm/addon-fit` 0.11.0, `lib/addon-fit.js` | MIT (same authors) |
| `../../../css/rdockpanel/xterm.css` | `@xterm/xterm` 6.0.0, `css/xterm.css` | MIT |
| `csp-shim.js` | RustCMS-Core W6 spike (2026-10-18) | this project |

To update: `npm pack @xterm/xterm@<v> @xterm/addon-fit@<v>` in an empty directory,
copy the files above, and run the terminal's Playwright check (no CSP violations).
