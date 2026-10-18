# RDockPanel

A [RustCMS](https://irisvn.github.io/RustCMS/) application. Commit `Cargo.lock`: it pins the RustCMS commit and every dependency.

Database: MySQL 8.4 everywhere (dev, tests, CI, production), from `docker-compose.yml`.

```sh
docker compose up -d     # MySQL 8.4 on 127.0.0.1:3306 (PANEL_MYSQL_PORT to change)
rustcms migrate          # create tables (DATABASE_URL in .env)
rustcms db:seed          # run src/database/seeders (DatabaseSeeder)
rustcms serve            # http://localhost:8000/health, the admin panel at /admin
RUSTCMS_TEST_MYSQL=mysql://root:root@127.0.0.1:3306/mysql RUSTCMS_TEST_BACKENDS=mysql cargo test   # PANEL_TEST_DOCKER=1 also runs against the real Docker engine
```

## Where things live

| Path | Holds | Generator |
|---|---|---|
| `src/bootstrap/` | `build()`: the app from its providers (`providers.rs` lists them) | |
| `src/providers/` | auth (permissions), admin panel (resources, pages), app (jobs, seeders, commands) | |
| `src/routes/api.rs` | every API route and its access | `make:api` |
| `src/http/` | `controllers/`, `requests/`, `resources/` (JSON) | `make:api` |
| `src/models/` | SeaORM entities and query scopes | `make:model` |
| `src/services/` | business rules shared by the panel, the API, jobs and seeders | `make:service` |
| `src/policies/` | who may do what | `make:policy` |
| `src/jobs/` | background jobs | `make:job` |
| `src/settings/` | settings stored in the database | `make:settings` |
| `src/console/` | console commands | `make:command` |
| `src/admin/` | `resources/` (one per model), `pages/` (settings pages) | `make:resource`, `make:settings` |
| `src/database/` | `migrations/`, `seeders/`, `factories/` | `make:migration`, `make:seeder`, `make:factory` |
| `tests/feature/` | tests through HTTP, on every database | `make:test` |
| `config/`, `lang/`, `storage/` | configuration, translations, uploaded files | |
