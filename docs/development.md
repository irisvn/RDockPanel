# Development

## Environment

- Rust stable, Docker (Docker Desktop, or OrbStack on macOS), the `rustcms` CLI.
- MySQL 8.4 from `docker-compose.yml` (`docker compose up -d`), with the same server flags as
  production (`utf8mb4_0900_bin`, UTC). The panel uses MySQL everywhere: development, tests, CI.
- `.env` holds `APP_KEY` (`rustcms key:generate`) and `DATABASE_URL`.

```sh
docker compose up -d && rustcms migrate && rustcms permission:sync
rustcms make:user --email admin@example.com --name Admin --super-admin
sh scripts/dev-swarm.sh
rustcms serve
```

Open the panel at http://localhost:8000/admin, the `APP_URL` host: sign-in checks the request's
origin, so `127.0.0.1` is refused.

Settings beyond `.env` use `RUSTCMS_<SECTION>__<KEY>`, e.g. `RUSTCMS_APP__LISTEN=127.0.0.1:8001`.

## A development Swarm

`scripts/dev-swarm.sh` (run it again any time) makes the local engine a single-node Swarm and runs
Traefik as the service `rdp-traefik` on ports 8080 and 8443, reading routes from
`storage/traefik/dynamic` (the panel's `[panel] traefik_dir`). `.env` sets
`RUSTCMS_PANEL__HTTPS_PORT=8443` so HTTP is redirected there. `docker swarm leave --force` undoes it;
other containers keep running.

## Tests

| Variable | Effect |
|---|---|
| `RUSTCMS_TEST_MYSQL=mysql://root:root@127.0.0.1:3306/mysql` | the test database server |
| `RUSTCMS_TEST_BACKENDS=mysql` | run database tests on MySQL only (required) |
| `PANEL_TEST_SWARM=1` | also run the Swarm tests (needs `scripts/dev-swarm.sh`: a Swarm manager and Traefik on 8080/8443): an overlay network and a service created, updated, removed; deployments; domains routed through Traefik, an image change under load without a failed request, HTTPS, projects isolated |
| `PANEL_TEST_TEMPLATES=1` | runs every real template's generator (network; about 15 minutes) and requires 95% to plan |
| `PANEL_TEST_ALL_DATABASES=1` | with `PANEL_TEST_SWARM=1`, the database test runs MySQL, MariaDB and MongoDB too (large images; CI runs PostgreSQL and Redis) |
| `PANEL_TEST_DOCKER=1` | also run against the real Docker engine: pulls `alpine:3.20`, runs a container labelled `rdockpanel.test=1`, removes it |

- Feature tests (`tests/feature/`) drive the app through HTTP with a `FakeDocker`
  (`tests/support/fake_docker.rs`): containers and networks whose ids differ from their names, as on a
  real engine, and a Traefik service. Use `support::app_with_docker(db, fake)` and
  `support::user_with(&app, name, perms)`. Every test app writes its routes to a temporary directory
  of its own (`support::scratch_traefik`): an app's boot rewrites its route directory from its
  database, so tests must never use `storage/traefik/dynamic` with pruning on. The one exception is
the real routing test (`routing.rs`), which writes there for the development Traefik, pruning off.
- Deployments run in tests by calling `deployment_service::run(state, id, FAST)` with short timings.
- Tests that change a real Swarm's networks hold `support::swarm_lock()`: Swarm's address allocator
  rejects tasks for a moment when networks come and go in parallel.
- Every admin route must have a case in `docker_routes::every_route_checks_its_permission`
  (`assert_access` fails otherwise).
- Live streams and the terminal are tested through a real listener (`tests/feature/live.rs`,
  `terminal.rs`).
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, then the tests with `set -o pipefail`
  if you pipe their output.

If MySQL tests fail with `PoolTimedOut`, the Docker daemon may be stuck: restart Docker/OrbStack.

## Front end

**The apps interface** (Dashboard, a project, a service and its tabs) is a React page on the
RustCMS extension host v2:
- **Sources:** `resources/admin/src/rdockpanel/dashboard.tsx` is the entry; `apps/` holds the views
  and `lib/` the shared pieces (`resources.ts` calls the resources' API with the CSRF token, `ui.tsx`
  has tabs, status dots and fields, `i18n.ts` the Vietnamese catalog).
- **Build:** `cd resources/admin && npm ci && npm run typecheck && npm run build` (Node 22). Commit
  the built `js/rdockpanel/dashboard.js`, `css/rdockpanel/dashboard.css`, `.rustcms-build.json` and
  `package-lock.json`: `cargo build` reads the built files and needs no Node. CI's `frontend` job
  checks the committed bundle is up to date.
- **Styles:** `dashboard.css`, theme variables and the panel's status and terminal tokens (light and
  dark), `rdp2-` prefix; the rules are [ui-guidelines.md](ui-guidelines.md).

**Every page is React** (`resources/admin/src/rdockpanel/`): the apps interface, Domains, Actions,
Settings, and the Docker pages (`docker/`: Containers, Images, Volumes, Networks, and the service
record's live aside `ServiceLive`). Logs and terminals use the shared viewers in `lib/viewers.tsx`.

xterm.js is vendored in `resources/admin/js/rdockpanel/vendor/` (see its README to update it).

**Design rules:** every page follows [ui-guidelines.md](ui-guidelines.md) (checklist in its
§10); the redesign that applied them (R0–R6) is done and recorded in [ui-redesign.md](ui-redesign.md).
- **Screenshots:** `RDP_SHOT_PASSWORD=… node scripts/ui-shots.mjs [dir]` signs in to a running panel
  (`RDP_SHOT_BASE`, default `http://localhost:8000/admin`; user `RDP_SHOT_EMAIL`, a super admin)
  and takes every page at 1440, 768 and 390px, light and dark, English and Vietnamese
  (`RDP_SHOT_PAGES`, `RDP_SHOT_WIDTHS`, `RDP_SHOT_THEMES`, `RDP_SHOT_LOCALES` narrow it). It needs
  Google Chrome and `npm ci` in `resources/admin`; the demo project (`demo` with `web`, `db`,
  `cache`) gives the pages content.
- **Browser dialogs:** `scripts/check-dialogs.sh` (in CI) fails when `window.confirm`, `alert` or
  `prompt` are used more than its `MAX`, lowered as pages move to `ConfirmDialog`.

## RustCMS dependency

`Cargo.toml` pins RustCMS-Core by a release tag (`v0.10.3`), or by git rev between releases. Never
use a plain path dependency on a working checkout: it picks up unfinished framework changes. For
unpushed framework commits, pin a `file://` git rev and switch back to GitHub once pushed. After an
upgrade, run `rustcms migrate` (framework tables) and `rustcms permission:sync`. Move with
`rustcms upgrade`, then `rustcms make:sdk` and `npm run build` in `resources/admin`. It also sets
`[package.metadata.rustcms] version`: that is the RustCMS version, not the panel's.

The RustCMS scheduler (`app.schedule`) runs only in `serve`; `rustcms schedule:list` shows the
panel's schedules.

## Conventions

- Follow the RustCMS layout and generators (`rustcms make:service`, `make:model -m`, `make:job`,
  `make:page --script`).
- Comments explain why; doc comments on public items.
- **Migrations only add**: new tables, nullable or defaulted columns; never a rename, a drop or a
  stricter constraint in the same release. Settings → Version's Update rolls a failing release
  back, and the previous version starts on the database as the new one left it (`migrate
  --allow-newer`): it must still be one it can use.
- **A release for a big change**: bump `Cargo.toml`, write its `## X.Y.Z` section of
  `CHANGELOG.md` (the GitHub Release's notes), push, let CI and the image build, then push the tag
  `vX.Y.Z` (`release.yml` names the image and publishes the Release). The docs follow the code in
  the same commit.
- Commit messages: short, in Vietnamese or English, describing the change.
- Changes are reviewed (Codex review in this project) before they are committed; CI must be green.
