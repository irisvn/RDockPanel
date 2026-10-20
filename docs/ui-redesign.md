# UI redesign plan

Applies [ui-guidelines.md](ui-guidelines.md) to every page. Status: **approved 2026-10-20**, in progress.
Screenshots of today's pages: 49 shots taken 2026-10-20 (1440px, 390px), audited page by page.

## What is wrong today (the audit, condensed)

1. Two UIs: the Docker pages (Containers, Images, Volumes, Networks) are plain JavaScript with
   their own buttons, badges, tables and radii (`panel.css`), and break at phone width.
2. Page headers differ on every page; project and service pages have no title; Settings has a
   row of `▾` menus between its tabs and its content.
3. 113 buttons: 42 small, 71 default, chosen per page (`Templates` is small on the Dashboard and
   default on a project); `+` on half the labels; delete looks like edit; text glyphs and an
   emoji as icons.
4. Ten font sizes, three monospace styles; metric values at two sizes; section titles at h2, h3
   and h4.
5. Three table styles; rows misaligned (`td` turned into a flex box); five empty-state styles;
   30 `window.confirm`.
6. Hard-coded colors for statuses and the terminal (wrong in dark mode); statuses named
   `ok`/`succeeded`/`running` for the same thing.

## Phases

Each phase is one commit (or a few), reviewed by Codex, tested and shipped on its own. Every
phase moves its own pages' confirmations, empty/loading/error states and statuses to the rules:
nothing is left for a final clean-up.

### R0. Contracts (documents and a Core request, no UI change)
- Status words per kind (guidelines §4) mapped from every backend state (`services.status`,
  deployments, actions, containers, tunnel); the mapping lives in one `lib/status.ts`.
- Breadcrumbs: ask RustCMS for structured breadcrumbs and a heading switch (guidelines §11);
  meanwhile pages keep their path links in the eyebrow.
- The Docker log viewer and terminal: list what the plain-JS versions do (line limit, Follow,
  connection state, bounded output; resize, New session, backpressure) so the React versions
  keep all of it.
- A browser test script in the repo (`scripts/ui-shots.mjs`, from today's screenshot script):
  seeded demo data, fixed clock, 390/768/1440px, light/dark, English/Vietnamese, used by every
  phase for before/after shots.
- CI: count `window.confirm|alert|prompt` in owned sources (not vendor or built bundles);
  the count may only go down, and must reach 0 at the end.

### R1. Foundations, piloted on one page
- `dashboard.css`: status and terminal tokens (light/dark), the type scale, `rdp-mono`; the ad-hoc
  font sizes and hex colors removed.
- `lib/`: `Stack`, `Section`, `ScrollTabs`, `TableCard` (with pagination), `RowActions`,
  `IconButton`, `StatusBadge`, `Metric`, `Facts`, `EmptyState`, `ConfirmDialog`, `FilePicker`,
  `ColorField`, `icons`, `format` (Intl numbers, sizes, dates).
- Pilot: the **Domains** page moves to them in the same commit (it has a table, icon buttons, an
  empty state and a dialog), so the components are proven on a real page before the rest.
- The shared CSS changes look on other pages too: before/after shots of every page in this
  phase, reviewed.

### R2. Dashboard, project page, Actions
- **Dashboard**: `PageHeader`, `Metric` tiles; each project a section with service cards (links,
  status dot and word); `Templates` in the header asks for the project first.
- **Project page**: eyebrow `Dashboard /` as a link, title the project, tabs `Services`, `Members`.
- **Actions**: `TableCard`, labeled `Latest 100 actions` until the server paginates (or server
  pagination added here, decided in R0).

### R3. Service page
- Header: eyebrow `Dashboard / demo` (links) and the kind, title `web` + `StatusBadge`; actions
  `[Open] [Restart] [⋯] [Deploy]` (`Start` instead of `Restart` when stopped); `⋯` holds
  `Stop`, `Force rebuild` and, on a phone, the hidden ones. Deleting stays in Advanced → Danger
  zone, confirmed by typing the name.
- Tabs in `ScrollTabs`; every tab a `Stack` of `Section`s.
- Overview: `Facts`; Monitor as `Metric`s with skeletons; deployment log on terminal tokens.
- Lists → `TableCard` + `RowActions`; every "add" a dialog; Roll back in the row menu.
- Environment and credentials follow the three secret kinds (guidelines §5).

### R4. Settings
- Header eyebrow `Settings`; tabs right under it.
- Panel tab: the `▾` menus become a **System** section: one row each for RDockPanel, Server,
  Traefik and Docker with its state, a visible `Logs` and its main action, the rest in `⋯`;
  restarts and reboots show their progress and time limit there.
- Every list (storage providers, channels, resolvers, certificates, Git and API tokens) in
  `TableCard`; dialog footers `[Cancel] [Primary]`; write-only secrets as `Kept`.
- Branding: `FilePicker`, `ColorField`, Save at the end.

### R5. Docker pages in React (five steps)
1. **Shared viewers**: endpoint-configurable `LogViewer` and `Terminal` keeping every behavior
   listed in R0; the service page switches to them; the service's live slot (`service-live.js`)
   moves to React.
2. **Containers**: all containers by default, a visible `Show N old tasks` toggle; the name opens
   details (inspect, stats, logs); one labeled action per state (Start / Stop / Unpause) plus `⋯`
   (Restart, Pause, Remove with force when needed); logs and terminal as icon buttons with
   tooltips.
3. **Images**: pull with progress (an action), remove with the force question kept.
4. **Volumes and Networks**: built-in networks protected as today.
5. Only then `panel.js`, `panel.css` and the old scripts are removed, with their registrations in
   `admin_panel_provider.rs`, after a search shows nothing else uses them.

### R6. Screenshots and docs
- New README screenshots from `scripts/ui-shots.mjs`; the confirm/alert count check at 0.

## Tests for every phase
- The full suite (MySQL, Docker, Swarm) and the frontend job.
- `scripts/ui-shots.mjs` before/after at 390/768/1440px, light/dark, English/Vietnamese, looked
  at page by page; keyboard checks of the changed dialogs, tabs and menus.
- Behaviour kept: every action, permission (disabled with a reason), deep link and test as before.

## Risks
- R5 is the biggest change; split in five steps, each shipped alone.
- Shared CSS (R1) changes every page's look at once: the pilot and full before/after shots
  catch regressions.
- Breadcrumbs depend on RustCMS; the eyebrow links are the fallback, not a blocker.

## Appendix: what the Docker viewers do today (kept in R5)

From `resources/admin/js/rdockpanel/containers.js`; the React `LogViewer` and `Terminal` (R5.1)
keep all of it, and the service page gets the same.

**Logs** (`/docker/containers/{id}/logs/follow/{n}`, server-sent events)
- A line count to load first: 100, 500 (default) or 2000; changing it reconnects.
- Follow on by default; off closes the stream (the lines stay).
- At most 5,000 lines kept; older ones dropped.
- Stays at the bottom only if the reader was already there.
- stderr lines marked; "No output." until the first line.
- After each (re)connect the server resends the last lines, so the view is cleared first.
- The connection state (connecting, reconnecting, closed) is shown.
- The React viewer adds: ANSI color codes stripped (as the Traefik logs do today).

**Stats** (`/docker/containers/{id}/stats`, events; running containers only): CPU %, memory
used/limit, network in/out, disk read/written.

**Terminal** (`/docker/containers/{id}/exec`, WebSocket; running containers with `docker.exec`)
- xterm.js with the fit addon, 5,000 lines of scrollback, monospace 13px; colors from the
  terminal tokens (today hard-coded `#0b0b0e`/`#e4e4e7`).
- No typing until the shell is open; the size is sent as `{"type":"resize"}` once open and on
  every resize (debounced 150ms).
- Input sent in chunks of 32 KiB; refused input is reported ("Some input was not sent").
- Output not yet drawn is capped at 8 MiB; past it the session closes with a reason.
- Never reconnects: a closed socket is the end of that shell; `New session` opens another.
- End reasons shown: the shell exited, idle 30 minutes, an error message, no longer allowed
  (4401/4403), disconnected.

**Inspect** (`GET /docker/containers/{id}`): the document as indented JSON.

**Deep links**: `?open=<id>&tab=logs|terminal|stats|inspect` opens a container's detail (used by
older links; kept as sub-paths `/p/containers/<id>/<tab>`, with the query still honoured).
