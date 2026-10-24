# RDockPanel — rules for working on this repository

RDockPanel is a Docker Swarm panel (EasyPanel-like) built as a **RustCMS application**: one Rust
binary (`rdockpanel`, axum on RustCMS-Core) with a React admin (`resources/admin`). Production and
every test run on **MySQL 8.4 only**. Start with `docs/architecture.md`; the other guides are in
`docs/` (api, development, install, permissions, security, firewall, ui-guidelines).

The person who owns this project writes in Vietnamese. Answer in Vietnamese, plainly.

## 1. Follow RustCMS-Core

Before introducing a kind of code you have not written here yet, read the RustCMS guide and its
reference apps: the layout and the examples, not only a snippet. In the RustCMS-Core repository
(locally `../RustCMS`) they are `docs/guide/src/**`, `docs/core/adr/0017-app-layout.md`,
`examples/demo` and `examples/lms`.

**Layout (ADR 0017).** Put each thing where RustCMS puts it:

| Folder | Holds |
|---|---|
| `src/bootstrap/` | the application from its providers, in order |
| `src/providers/` | auth (permissions), admin panel (resources, pages), app (bindings, jobs, schedules) |
| `src/routes/` | every route and its access (`admin.rs`, `api.rs`) |
| `src/http/controllers/` | handlers: parse, authorize, call a service, answer |
| `src/models/` | SeaORM entities and query scopes (functions next to the model) |
| `src/services/` | business rules shared by the panel, the API, jobs and seeders |
| `src/policies/` | resource and shared policies, helpers in `mod.rs`, custom permission names in `permissions.rs` |
| `src/settings/` | settings data (`impl Settings`); its `impl SettingsPage` goes in `src/admin/pages/` |
| `src/admin/resources/` | fields, forms, tables, scopes, hooks; `fn policy()` only returns the policy |
| `src/admin/pages/`, `src/admin/slots/` | Rust page and slot definitions; the React code is in `resources/admin/src/rdockpanel/` |
| `src/jobs/` | queued jobs |
| `src/database/` | `migrations/` (listed in `mod.rs`), `seeders/`, `factories/` |
| `tests/feature/` | one test binary (`main.rs` lists the modules); helpers in `tests/support/` |

- Rules that must hold whoever writes (a form, the API, a template, a job) live in **one service
  function**, called by every writer. Never repeat them in a hook or a controller.
- Use the `src/policies/permissions.rs` constants for the panel's custom permissions. Resource CRUD
  permissions follow RustCMS's `<slug>.<ability>`. Declare new permissions and their labels in
  `providers/auth_service_provider.rs`, then run `rustcms permission:sync`.
- The framework itself is changed in the RustCMS-Core repository, never patched here. `Cargo.toml`
  pins Core by its GitHub tag. A `file://` pin is only for Core commits not pushed yet: CI fails
  meanwhile, and pushing Core is the owner's decision.

**Framework guarantees, and RDockPanel conventions on top of them:**
- **Access**
  - Authorization is deny by default, and every route declares its access.
  - Every new admin route needs a case in `docker_routes::every_route_checks_its_permission`.
  - Field rules (readable, creatable, updatable) hold on the server; what the UI hides grants
    nothing. Use `guarded()` for fields only the server sets.
- **Policies**
  - Override `admits` so a caller without the ability is refused before any lookup (no existence
    probing).
  - Put record visibility in `Resource::scope()`. Return `false` from `view_depends_on_record()`
    only when that scope fully enforces visibility; otherwise keep the default.
  - Query with `ctx.conn()` in policies and hooks.
- **Input**: take JSON with `rustcms::Json` or `Validated<T>`. Malformed JSON is 400; a body of
  the wrong shape and field errors (`ValidationErrors`) are 422.
- **Writes**
  - Use `rustcms::db::begin_write` (MySQL: READ COMMITTED with the framework's lock wait).
  - Lock the row you decide on (`lock_exclusive`) before reading what the decision depends on.
  - Dispatch the jobs a write causes in that write's transaction. Keep job payloads to ids, with no
    secrets, and make retried effects idempotent.
- **Audit**
  - Database changes, writes inside resource actions included, are audited with
    `rustcms::audit::record` (or `ctx.audit`) in the change's own transaction.
  - `record_detached` is best effort: keep it for effects that cannot share the transaction, such
    as Docker calls.
  - RDockPanel: a read of secrets is audited first (no audit, no values).
- **Encryption** (`Encrypted<T>`)
  - A value holds at most 32 KiB: its JSON, `rustcms::crypto::MAX_PLAINTEXT`.
  - A sealed column outside admin fields must be registered with `app.encrypted_column`, so that
    `rustcms key:reencrypt` finds it.
  - RDockPanel: check the size with `deploy_rules_service::sealable` and answer 422. A service's
    files are sealed together when it deploys.
- **Columns**: MySQL's `TEXT` holds 64 KiB (bytes), so a longer value needs `MEDIUMTEXT` on MySQL.
  A field's `max_length` counts characters, and multi-byte text can overflow a byte-sized column.
- **Configuration**: read a section with `config.section("x")?`. A value of the wrong kind is an
  error, never a silent default.
- **Migrations**
  - Never edit or rename a migration that has run; append a new one to `mod.rs`.
  - RDockPanel: migrations only add (new tables, nullable or defaulted columns). No rename, drop or
    stricter constraint in the same release: a rolled-back release starts on the newer schema
    (`migrate --allow-newer`, see `docs/development.md`).
  - Keep `use_transaction() -> Some(true)` unless a statement cannot run in one.
  - MySQL commits each DDL statement, so make a migration safe to run again after a partial
    failure. A `down` that would lose data refuses before changing anything.

## 2. Follow the UI/UX guidelines

Every new or changed screen follows `docs/ui-guidelines.md`, and mockups too, so that the mockup is
what ships. Its **§10 checklist** is binding: run it before showing anything.
- Build with the kit (`@rustcms/admin-sdk/ui`: `Alert`, `Table`, `Button`, `Dialog`, `Skeleton`…)
  and the panel's `resources/admin/src/rdockpanel/lib/` before adding a CSS class.
- **Color**: theme tokens only, never a literal color. A status color fills a dot or a bar segment,
  never a background, a border or text.
- **Type and space**: the five text sizes (12, 14, 16, 20, 24 px), except code values, which use
  the 13 px `rdp2-mono`. Spacing stays on the 4 px grid.
- **Icons** come from `lib/icons`, never text glyphs such as `!` or `✕`.
- **Dialogs**
  - A visible Cancel or Close, `ghost`, in `rdp2-dialog-foot`, at the prescribed width.
  - Focus the first field (Cancel for a confirmation), return focus to the opener, and focus the
    first invalid field after a refusal.
  - Buttons say what they are doing while busy ("Saving…").
  - Never the browser's `confirm`/`alert`/`prompt` (`scripts/check-dialogs.sh` fails on them):
    use `useAsk`.
- **Feedback**: a toast follows an action. Long work shows pending, then queued, then done: never
  report success when the server only accepted the work.
- **Lists**: paginate long ones (25 rows).
- **Words** follow guidelines §9. Translate user-facing strings with `t()`: the frontend's go in
  `lib/i18n.ts`, server-side labels in `lang/vi.toml`.
- **Review**: show the owner a design before a big visual change.
- **Screenshots**: look at the real pages with `scripts/ui-shots.mjs`, run on a local panel with
  a test account in `RDP_SHOT_EMAIL` and `RDP_SHOT_PASSWORD`. Take the default widths (390, 768,
  1440), both themes and both languages.
  - It uses `playwright-core` with the installed Chrome, so nothing is downloaded.
  - Its page list is explicit: add new pages to it, and look separately at changed dialogs and at
    loading and error states.
- **Build**: run `npm run typecheck` and `npm run build` in `resources/admin`, and commit the built
  files. CI fails when `js/`, `css/` or `.rustcms-build.json` differ from a fresh build. A running
  `rustcms serve` keeps the old bundle until you restart it.

## 3. Test

- A behaviour change needs a regression test that **fails without the change**: check it once by
  undoing the change. Test HTTP behaviour through feature tests, with the shared helpers in
  `tests/support/`. Real Swarm network tests hold `support::swarm_lock()`.
- Before every commit, run the whole suite on MySQL (the optional switches are in
  `docs/development.md`). Gate the commit on this block's exit code, and keep failures visible when
  piping output:

  ```sh
  (
    set -e
    set -o pipefail
    docker compose up -d          # MySQL 8.4
    sh scripts/dev-swarm.sh       # Swarm + Traefik for the Swarm tests
    RUSTCMS_TEST_MYSQL="${RUSTCMS_TEST_MYSQL:?the local test MySQL URL, see docs/development.md}" \
      RUSTCMS_TEST_BACKENDS=mysql \
      PANEL_TEST_DOCKER=1 PANEL_TEST_SWARM=1 cargo test --locked
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
  )
  ```

- Do not use Nginx images for new demo or test services; prefer `ghcr.io/traefik/whoami`.
- If OrbStack's Docker hangs (tests fail with `PoolTimedOut`), ask the owner to restart it; do not
  restart it yourself.

## 4. Workflow

1. **Plan** a non-trivial change. For UI work, show the owner a mockup.
2. **Review**: have Codex review the plan, and the code before each commit (read-only), then fix
   what it finds. Report its findings faithfully.
3. **Docs**: update the affected documentation in the same commit (both READMEs when relevant,
   `docs/*.md`, `CHANGELOG.md`).
4. **Commits**: short messages in Vietnamese, easy to understand.
5. **Releases**: a big update bumps the version.
   - Bump `[package] version` in `Cargo.toml`: a minor bump for features, a patch bump for fixes.
     `[package.metadata.rustcms] version` is RustCMS's version: only `rustcms upgrade` changes it.
   - Add a CHANGELOG section: a few plain bullets on what changed for users, no internals.
   - Push to `main`, and wait for CI and that commit's image (`release.yml`) to succeed.
   - Then push the tag `vX.Y.Z`: it names that image and publishes the GitHub Release. It does not
     build anything.
6. **Test server**: update it to the release and check the real pages. The owner gives the access;
   never write passwords, tokens or account credentials into files, commits or notes.
7. **Clean up after each task** (the owner's machine is short on disk):
   - Run `cargo clean` and remove this task's `/tmp` files.
   - Stop the dev Swarm's services with
     `docker service scale demo_cache=0 demo_db=0 demo_web=0 rdp-traefik=0`. Swarm restarts
     containers that are only stopped. `scripts/dev-swarm.sh` does not scale them back up, so do it
     with `=1` when they are needed again.
   - Leave Docker itself running; do not stop it.

## 5. Shell notes (zsh on macOS)

- `for x in $list` does not split lines in zsh: use `${(f)list}` or `while read`.
- Give every wait loop a deadline. There is no `timeout` command: use
  `perl -e 'alarm N; exec @ARGV'`.
- Filter workflow runs by the full commit SHA (`gh run list --commit "$(git rev-parse HEAD)"`), and
  check `headSha` in the result.
