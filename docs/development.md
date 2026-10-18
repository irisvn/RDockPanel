# Development

## Environment

- Rust stable, Docker (Docker Desktop, or OrbStack on macOS), the `rustcms` CLI.
- MySQL 8.4 from `docker-compose.yml` (`docker compose up -d`), with the same server flags as
  production (`utf8mb4_0900_bin`, UTC). The panel uses MySQL everywhere: development, tests, CI.
- `.env` holds `APP_KEY` (`rustcms key:generate`) and `DATABASE_URL`.

```sh
docker compose up -d && rustcms migrate && rustcms permission:sync
rustcms make:user --email admin@example.com --name Admin --super-admin
rustcms serve
```

Settings beyond `.env` use `RUSTCMS_<SECTION>__<KEY>`, e.g. `RUSTCMS_APP__LISTEN=127.0.0.1:8001`.

## A development Swarm

`scripts/dev-swarm.sh` (run it again any time) makes the local engine a single-node Swarm and runs
Traefik as the service `rdp-traefik` on ports 8080 and 8443, reading routes from
`storage/traefik/dynamic`. `docker swarm leave --force` undoes it; other containers keep running.

## Tests

| Variable | Effect |
|---|---|
| `RUSTCMS_TEST_MYSQL=mysql://root:root@127.0.0.1:3306/mysql` | the test database server |
| `RUSTCMS_TEST_BACKENDS=mysql` | run database tests on MySQL only (required) |
| `PANEL_TEST_SWARM=1` | also run the Swarm tests (needs a Swarm manager: `scripts/dev-swarm.sh`): an overlay network and a service created, updated, removed |
| `PANEL_TEST_DOCKER=1` | also run against the real Docker engine: pulls `alpine:3.20`, runs a container labelled `rdockpanel.test=1`, removes it |

- Feature tests (`tests/feature/`) drive the app through HTTP with a `FakeDocker`
  (`tests/support/fake_docker.rs`): containers and networks whose ids differ from their names, as on a
  real engine. Use `support::app_with_docker(db, fake)` and `support::user_with(&app, name, perms)`.
- Every admin route must have a case in `docker_routes::every_route_checks_its_permission`
  (`assert_access` fails otherwise).
- Live streams and the terminal are tested through a real listener (`tests/feature/live.rs`,
  `terminal.rs`).
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, then the tests with `set -o pipefail`
  if you pipe their output.

If MySQL tests fail with `PoolTimedOut`, the Docker daemon may be stuck: restart Docker/OrbStack.

## Front end

The pages are plain JavaScript (`resources/admin/js/rdockpanel/`), loaded by the RustCMS panel with
`extension_script` and styled by `resources/admin/css/rdockpanel/panel.css` (theme variables, `rdp-`
prefix, no inline styles). Shared helpers are in `panel.js` (`window.rdockPanel`): `h` builds elements
(never `innerHTML`), `api` calls the routes with the CSRF token. Translations: `registerTranslations`
in `panel.js`. React pages come with the RustCMS extension host v2 (planned).

xterm.js is vendored in `resources/admin/js/rdockpanel/vendor/` (see its README to update it).

## RustCMS dependency

`Cargo.toml` pins RustCMS-Core by git rev. Never use a plain path dependency on a working checkout:
it picks up unfinished framework changes. For unpushed framework commits, pin a `file://` git rev
and switch back to GitHub once pushed.

## Conventions

- Follow the RustCMS layout and generators (`rustcms make:service`, `make:model -m`, `make:job`,
  `make:page --script`).
- Comments explain why; doc comments on public items.
- Commit messages: short, in Vietnamese or English, describing the change.
- Changes are reviewed (Codex review in this project) before they are committed; CI must be green.
