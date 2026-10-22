//! Services: apps deployed as Docker Swarm services from an image. What is
//! set here applies on the next deployment. Once deployed, the name and the
//! project are fixed and deleting needs Destroy first (it removes the Swarm
//! service).

use std::sync::Arc;

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::{DatabaseTransaction, EntityTrait, QuerySelect};

use crate::admin::{int, is_set, text};
use crate::jobs::publish_routes::PublishRoutes;
use crate::jobs::run_deployment::RunDeployment;
use crate::models::service::{self, NEW, privileged};
use crate::policies::permissions;
use crate::policies::service_policy::ServicePolicy;
use crate::services::build_service::{inside, valid_git_url, valid_ref};
use crate::services::deploy_rules_service::{valid_image_reference, valid_name};
use crate::services::deploy_service::{command_args, valid_env_name};
use crate::services::deployment_service::{self, Trigger};
use crate::services::rate_limit_service;

pub struct ServiceResource;

/// Replicas, CPU (thousandths of a core), memory (MB), health check bounds.
const REPLICAS: std::ops::RangeInclusive<i64> = 1..=50;
const CPU_MILLIS: std::ops::RangeInclusive<i64> = 10..=256_000;
const MEM_MB: std::ops::RangeInclusive<i64> = 4..=1_048_576;
const HEALTH_INTERVAL_S: std::ops::RangeInclusive<i64> = 1..=3600;
const HEALTH_RETRIES: std::ops::RangeInclusive<i64> = 1..=20;
/// A rate limit's average and burst.
const RATE: std::ops::RangeInclusive<i64> = 1..=rate_limit_service::MAX as i64;

#[async_trait]
impl Resource for ServiceResource {
    type Entity = service::Entity;
    const SLUG: &'static str = "services";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Service")
            .title("name")
            .icon("box")
            .group("Deploy")
            .navigation(false)
            .unique(
                "services_project_name_unique",
                &["project_id", "name"],
                "is already taken in this project",
            )
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(service::Column::ProjectId)
                .belongs_to("projects")
                .required()
                .filterable(),
            Field::column(service::Column::Name)
                .required()
                .searchable()
                .sortable()
                .max_length(40),
            // An image's; empty when the panel builds it (`source`).
            Field::column(service::Column::Image)
                .searchable()
                .max_length(255),
            Field::column(service::Column::Source),
            Field::column(service::Column::GitUrl).max_length(512),
            Field::column(service::Column::GitProviderId),
            Field::column(service::Column::GitBranch).max_length(255),
            Field::column(service::Column::GitPath).max_length(255),
            Field::column(service::Column::Builder),
            Field::column(service::Column::DockerfilePath).max_length(255),
            Field::column(service::Column::Dockerfile).max_length(65_535),
            // A Compose service's file (written here), or its path in the repository.
            Field::column(service::Column::Compose)
                .max_length(crate::services::compose_service::MAX_FILE),
            Field::column(service::Column::ComposeFile).max_length(255),
            Field::column(service::Column::Notes).max_length(20_000),
            Field::column(service::Column::CapAdd).max_length(1000),
            Field::column(service::Column::CapDrop).max_length(1000),
            Field::column(service::Column::Sysctls).max_length(4000),
            Field::column(service::Column::Groups).max_length(1000),
            Field::column(service::Column::Maintenance),
            Field::column(service::Column::MaintenanceTitle).max_length(200),
            Field::column(service::Column::MaintenanceSubtitle).max_length(1000),
            Field::column(service::Column::MaintenanceEta),
            Field::column(service::Column::MaintenanceEtaOffset),
            Field::column(service::Column::MaintenanceContact).max_length(255),
            Field::column(service::Column::MaintenancePage),
            Field::column(service::Column::MaintenanceHtml)
                .max_length(crate::services::maintenance_service::MAX_HTML),
            Field::column(service::Column::MaintenanceBypass).max_length(4000),
            Field::column(service::Column::RateLimitAverage),
            Field::column(service::Column::RateLimitPeriod),
            Field::column(service::Column::RateLimitBurst),
            Field::column(service::Column::RateLimitExempt).max_length(4000),
            Field::column(service::Column::Command)
                .max_length(crate::services::deploy_rules_service::MAX_COMMAND),
            Field::column(service::Column::Replicas).required(),
            Field::column(service::Column::CpuLimitMillis),
            Field::column(service::Column::MemLimitMb),
            Field::column(service::Column::CpuReservationMillis),
            Field::column(service::Column::MemReservationMb),
            Field::column(service::Column::ZeroDowntime),
            Field::column(service::Column::HealthcheckCmd).max_length(500),
            Field::column(service::Column::HealthcheckIntervalS),
            Field::column(service::Column::HealthcheckRetries),
            // Sealed with APP_KEY; names shown, values never.
            Field::column(service::Column::Env)
                .encrypted_map()
                .max_items(100)
                .max_length(16_384),
            Field::column(service::Column::RegistryUsername).max_length(255),
            Field::column(service::Column::RegistryPassword).encrypted(),
            // Set by deployments, never by the form.
            Field::column(service::Column::DesiredState)
                .guarded()
                .readable(|_| true)
                .filterable(),
            Field::column(service::Column::Status)
                .guarded()
                .readable(|_| true)
                .filterable()
                .sortable(),
            Field::column(service::Column::DeployedAt)
                .guarded()
                .readable(|_| true)
                .sortable(),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            SelectInput::make("project_id"),
            TextInput::make("name")
                .help("Lowercase letters, digits and dashes; the service is <project>_<name>"),
            TextInput::make("image")
                .column_span(2)
                .help("e.g. nginx:1.27 or ghcr.io/org/app:v2"),
            TextInput::make("command")
                .column_span(2)
                .help("Overrides the image's command (optional)"),
            NumberInput::make("replicas"),
            Toggle::make("zero_downtime")
                .help("Start the new task before stopping the old one (needs a health check)"),
            NumberInput::make("cpu_limit_millis").help("1000 = one core; empty: no limit"),
            NumberInput::make("mem_limit_mb").help("MB; empty: no limit"),
            NumberInput::make("cpu_reservation_millis"),
            NumberInput::make("mem_reservation_mb"),
            TextInput::make("healthcheck_cmd")
                .column_span(2)
                .help("Run in the container, e.g. wget -qO- http://127.0.0.1/health"),
            NumberInput::make("healthcheck_interval_s"),
            NumberInput::make("healthcheck_retries"),
            SecretMapInput::make("env")
                .column_span(2)
                .help("Environment variables: names are letters, digits and _; values are never shown again"),
            TextInput::make("registry_username")
                .help("For a private image: the registry is the image's (e.g. ghcr.io)"),
            SecretInput::make("registry_password").help("Or an access token"),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("name"),
                TextColumn::make("project_id"),
                TextColumn::make("image"),
                TextColumn::make("replicas"),
                TextColumn::make("status"),
            ])
            .filters(vec![SelectFilter::make("project_id")])
            .default_sort("name")
    }

    /// Deploy, start, stop, restart (`services.deploy`) and destroy
    /// (`services.destroy`): each asks for a deployment, run in the background.
    fn actions() -> Vec<Action<service::Entity>> {
        vec![
            deployment_action("deploy", Trigger::Deploy, "Deploy").icon("box"),
            Action::<service::Entity>::new("rebuild", |ctx, txn, services, _| {
                Box::pin(async move {
                    let user = ctx.principal.id.parse().ok();
                    for service in services {
                        let row = deployment_service::request_rebuild(
                            txn,
                            &service,
                            user,
                            ctx.keyring()?,
                        )
                        .await?;
                        ctx.dispatch(
                            txn,
                            &RunDeployment {
                                deployment_id: row.id,
                            },
                        )
                        .await?;
                    }
                    Ok(())
                })
            })
            .label("Force rebuild")
            .ability("deploy"),
            deployment_action("start", Trigger::Start, "Start"),
            deployment_action("stop", Trigger::Stop, "Stop")
                .requires_confirmation("Stop this service? Its tasks are removed."),
            deployment_action("restart", Trigger::Restart, "Restart"),
            Action::<service::Entity>::new("republish", |ctx, txn, services, _| {
                Box::pin(async move {
                    for service in services {
                        ctx.dispatch(
                            txn,
                            &PublishRoutes {
                                service_id: service.id,
                            },
                        )
                        .await?;
                    }
                    Ok(())
                })
            })
            .label("Republish routes")
            .ability("deploy"),
            deployment_action("destroy", Trigger::Destroy, "Destroy")
                .ability("destroy")
                .color("danger")
                .requires_confirmation("Remove this service from Docker? Its volumes are kept."),
        ]
    }

    fn relations() -> Vec<Relation> {
        vec![
            Relation::has_many("domains", "service_domains", "service_id").label("Domains"),
            Relation::has_many("ports", "service_ports", "service_id").label("Ports"),
            Relation::has_many("mounts", "service_mounts", "service_id")
                .label("Volumes and mounts"),
            Relation::has_many("deployments", "deployments", "service_id").label("Deployments"),
        ]
    }

    fn policy() -> Arc<dyn Policy> {
        Arc::new(ServicePolicy)
    }

    /// A member sees their projects' records only (a role seeing every
    /// record: all of them).
    fn scope(
        query: sea_orm::Select<<Self as Resource>::Entity>,
        ctx: &Ctx,
    ) -> sea_orm::Select<<Self as Resource>::Entity> {
        use sea_orm::{ColumnTrait, QueryFilter};
        match crate::policies::members_only(ctx, "services") {
            Some(user) => query.filter(
                crate::models::service::Column::ProjectId
                    .in_subquery(crate::services::access_service::member_projects(user)),
            ),
            None => query,
        }
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        let mut data = normalize(data);
        // Into a project where the caller adds services (Core asks the
        // policy without the record).
        let project = int(&data, "project_id").ok_or(rustcms::AppError::Forbidden)?;
        if !crate::services::access_service::can(txn, &ctx.principal, project, "services.create")
            .await?
        {
            return Err(rustcms::AppError::Forbidden);
        }
        check(ctx, &data, None)?;
        check_git_provider(ctx, txn, &data, None).await?;
        // Built by the panel: no image to name (the column holds one).
        if !data.contains_key("image") {
            data.insert("image".to_owned(), serde_json::Value::String(String::new()));
        }
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &service::Model,
        data: Data,
    ) -> AppResult<Data> {
        let data = normalize(data);
        // Read again, locked: a deployment asked for meanwhile waits for
        // this edit, or went first (then its status is checked here).
        let record = service::Entity::find_by_id(record.id)
            .lock_exclusive()
            .one(txn)
            .await?
            .ok_or(rustcms::AppError::NotFound)?;
        // A Compose file runs anything on the host, as a service with added
        // capabilities or sysctls does: its administrators only.
        if (record.kind == "compose" || privileged(&record)) && !ctx.can(permissions::SYSTEM_MANAGE)
        {
            return Err(rustcms::AppError::Forbidden);
        }
        // Moved (before its first deployment): where it lands, too.
        if let Some(project) = int(&data, "project_id")
            && project != record.project_id
            && !crate::services::access_service::can(
                txn,
                &ctx.principal,
                project,
                "services.create",
            )
            .await?
        {
            return Err(rustcms::AppError::Forbidden);
        }
        check(ctx, &data, Some(&record))?;
        check_git_provider(ctx, txn, &data, Some(&record)).await?;
        Ok(data)
    }

    /// Its maintenance (and who it lets through) and rate limit applied
    /// without a deployment: its routes written again (once this commits).
    async fn after_save(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &service::Model,
    ) -> AppResult<()> {
        ctx.dispatch(
            txn,
            &crate::jobs::publish_routes::PublishRoutes {
                service_id: record.id,
            },
        )
        .await?;
        Ok(())
    }
}

/// A GitHub token (Settings → Git) clones a repository of GitHub, for the
/// projects it is given to. Checked when the repository or the token
/// changes (a token taken from the project later fails its deployments).
async fn check_git_provider(
    ctx: &Ctx,
    txn: &DatabaseTransaction,
    data: &Data,
    record: Option<&service::Model>,
) -> AppResult<()> {
    if !["git_provider_id", "git_url", "source", "project_id"]
        .iter()
        .any(|f| data.contains_key(*f))
    {
        return Ok(());
    }
    let provider = if data.contains_key("git_provider_id") {
        int(data, "git_provider_id")
    } else {
        record.and_then(|r| r.git_provider_id)
    };
    let source = text(data, "source")
        .map(str::to_owned)
        .or(record.map(|r| r.source.clone()))
        .unwrap_or_default();
    let (Some(provider), "git") = (provider, source.as_str()) else {
        return Ok(());
    };
    let project = int(data, "project_id").or(record.map(|r| r.project_id));
    // Held until the write commits: a deletion of the token waits for it
    // (and then counts this service), or went first.
    let found = crate::models::git_provider::Entity::find_by_id(provider)
        .lock_shared()
        .one(txn)
        .await?;
    if !found.is_some_and(|p| project.is_some_and(|project| p.allows(project))) {
        return Err(ValidationErrors::field(
            "git_provider_id",
            ctx.t("is not a GitHub token given to this project (Settings → Git)"),
        )
        .into());
    }
    let url = if data.contains_key("git_url") {
        text(data, "git_url").map(str::to_owned)
    } else {
        record.and_then(|r| r.git_url.clone())
    };
    if url
        .as_deref()
        .and_then(crate::services::github_service::repository_of)
        .is_none()
    {
        return Err(ValidationErrors::field(
            "git_url",
            ctx.t("must be https://github.com/<owner>/<repo> with a GitHub token"),
        )
        .into());
    }
    Ok(())
}

/// The image as Docker takes it (no surrounding spaces); exempt addresses
/// canonical, one a line (refused ones left as given: [`check`] says why).
fn normalize(mut data: Data) -> Data {
    if let Some(image) = text(&data, "image").map(|i| i.trim().to_owned()) {
        data.insert("image".to_owned(), serde_json::Value::String(image));
    }
    for field in ["rate_limit_exempt", "maintenance_bypass"] {
        if let Some(Ok(list)) = text(&data, field).map(|e| rate_limit_service::exempt(Some(e))) {
            let value = if list.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(list.join("\n"))
            };
            data.insert(field.to_owned(), value);
        }
    }
    if let Some(contact) = text(&data, "maintenance_contact").map(|c| c.trim().to_owned()) {
        let value = if contact.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(contact)
        };
        data.insert("maintenance_contact".to_owned(), value);
    }
    data
}

fn check(ctx: &Ctx, data: &Data, record: Option<&service::Model>) -> AppResult<()> {
    let mut errors = ValidationErrors::new();
    // New environment variables (`{"set": {NAME: value}}`): names a shell takes.
    if let Some(set) = data
        .get("env")
        .and_then(|e| e.get("set"))
        .and_then(|s| s.as_object())
        && set.keys().any(|name| !valid_env_name(name))
    {
        errors.add(
            "env",
            ctx.t("names must be letters, digits and _, not starting with a digit"),
        );
    }
    if let Some(name) = text(data, "name")
        && !valid_name(name)
    {
        errors.add(
            "name",
            ctx.t("must be 1-40 lowercase letters, digits and dashes, starting with a letter"),
        );
    }
    if let Some(command) = text(data, "command")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        && command_args(command).is_none()
    {
        errors.add("command", ctx.t("has an unclosed quote"));
    }
    // Where the image comes from, as it will be after this write.
    let after = |field: &str, old: Option<&str>| -> Option<String> {
        if data.contains_key(field) {
            text(data, field).map(str::to_owned)
        } else {
            old.map(str::to_owned)
        }
    };
    let source =
        after("source", record.map(|r| r.source.as_str())).unwrap_or_else(|| "image".to_owned());
    // Capabilities and sysctls: added ones reach the host's kernel, for the
    // panel's administrators only (dropping is anyone's).
    let system = ctx.can(permissions::SYSTEM_MANAGE);
    for field in ["cap_add", "cap_drop"] {
        if let Some(caps) = text(data, field) {
            if !crate::services::deploy_service::valid_capabilities(caps) {
                errors.add(field, ctx.t("must be capability names like NET_ADMIN"));
            } else if field == "cap_add" && !caps.trim().is_empty() && !system {
                errors.add(field, ctx.t("needs system.manage"));
            }
        }
    }
    // Supplemental groups (a group may open devices or sockets).
    if let Some(list) = text(data, "groups") {
        let words: Vec<&str> = list
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .collect();
        if !words
            .iter()
            .all(|g| crate::services::deploy_service::valid_group(g))
        {
            errors.add("groups", ctx.t("must be group names or ids"));
        } else if !words.is_empty() && !system {
            errors.add("groups", ctx.t("needs system.manage"));
        }
    }
    if let Some(lines) = text(data, "sysctls") {
        if crate::services::deploy_service::sysctls(Some(lines)).is_none() {
            errors.add("sysctls", ctx.t("must be key=value lines"));
        } else if !lines.trim().is_empty() && !system {
            errors.add("sysctls", ctx.t("needs system.manage"));
        }
    }
    let compose = record.is_some_and(|r| r.kind == "compose");
    if compose {
        // Its file written here (`image`) or its repository's (`git`).
        if !matches!(source.as_str(), "image" | "git") {
            errors.add(
                "source",
                ctx.t("a Compose file is written here or comes from Git"),
            );
        }
        if let (Some(text), Some(r)) = (text(data, "compose"), record)
            && !text.trim().is_empty()
            && let Err(e) = crate::services::compose_service::prepare(text, "p", &r.name)
        {
            errors.add("compose", e);
        }
        if text(data, "compose_file").is_some_and(|p| inside(p).is_none()) {
            errors.add("compose_file", ctx.t("must stay inside the repository"));
        }
    }
    match source.as_str() {
        "image" if compose => {}
        "image" => {
            if !after("image", record.map(|r| r.image.as_str()))
                .is_some_and(|i| valid_image_reference(i.trim()))
            {
                errors.add("image", ctx.t("is not an image reference"));
            }
        }
        "git" => {
            if !after("git_url", record.and_then(|r| r.git_url.as_deref()))
                .is_some_and(|u| valid_git_url(&u))
            {
                errors.add("git_url", ctx.t("must be an https or ssh repository URL"));
            }
            if after("git_branch", record.and_then(|r| r.git_branch.as_deref()))
                .is_some_and(|b| !b.trim().is_empty() && !valid_ref(b.trim()))
            {
                errors.add("git_branch", ctx.t("is not a branch, tag or commit"));
            }
            for field in ["git_path", "dockerfile_path"] {
                if text(data, field).is_some_and(|p| inside(p).is_none()) {
                    errors.add(field, ctx.t("must stay inside the repository"));
                }
            }
        }
        "dockerfile" => {
            if !after("dockerfile", record.and_then(|r| r.dockerfile.as_deref()))
                .is_some_and(|d| !d.trim().is_empty())
            {
                errors.add("dockerfile", ctx.t("is required"));
            }
        }
        _ => errors.add("source", ctx.t("must be image, git or dockerfile")),
    }
    // A database runs its engine's image (its volume and check are the engine's).
    if record.is_some_and(|r| !matches!(r.kind.as_str(), "app" | "compose")) && source != "image" {
        errors.add("source", ctx.t("a database runs its engine's image"));
    }
    if text(data, "dockerfile").is_some_and(|d| d.len() > 65_535) {
        errors.add("dockerfile", ctx.t("is at most 64 KB"));
    }
    if let Some(builder) = text(data, "builder")
        && builder != "dockerfile"
    {
        errors.add("builder", ctx.t("must be dockerfile"));
    }
    let range =
        |field: &str, bounds: std::ops::RangeInclusive<i64>, errors: &mut ValidationErrors| {
            if is_set(data, field) && !int(data, field).is_some_and(|n| bounds.contains(&n)) {
                errors.add(
                    field,
                    format!(
                        "{} {}–{}",
                        ctx.t("must be between"),
                        bounds.start(),
                        bounds.end()
                    ),
                );
            }
        };
    range("replicas", REPLICAS, &mut errors);
    range("cpu_limit_millis", CPU_MILLIS, &mut errors);
    range("cpu_reservation_millis", CPU_MILLIS, &mut errors);
    range("mem_limit_mb", MEM_MB, &mut errors);
    range("mem_reservation_mb", MEM_MB, &mut errors);
    range("healthcheck_interval_s", HEALTH_INTERVAL_S, &mut errors);
    range("healthcheck_retries", HEALTH_RETRIES, &mut errors);
    range("rate_limit_average", RATE, &mut errors);
    range("rate_limit_burst", RATE, &mut errors);
    if let Some(period) = text(data, "rate_limit_period")
        && !rate_limit_service::PERIODS.contains(&period)
    {
        errors.add("rate_limit_period", ctx.t("must be 1s, 1m or 1h"));
    }
    for field in ["rate_limit_exempt", "maintenance_bypass"] {
        if let Some(Err(why)) = text(data, field).map(|e| rate_limit_service::exempt(Some(e))) {
            errors.add(field, why);
        }
    }
    if let Some(contact) = text(data, "maintenance_contact")
        && !contact.trim().is_empty()
        && !crate::services::maintenance_service::valid_contact(contact)
    {
        errors.add(
            "maintenance_contact",
            ctx.t("must be an email or an https link"),
        );
    }
    if text(data, "maintenance_html")
        .is_some_and(|h| h.len() > crate::services::maintenance_service::MAX_HTML)
    {
        errors.add("maintenance_html", ctx.t("is larger than 256 KB"));
    }
    if let Some(page) = text(data, "maintenance_page")
        && !matches!(page, "default" | "custom")
    {
        errors.add("maintenance_page", ctx.t("must be default or custom"));
    }
    range("maintenance_eta_offset", -840..=840, &mut errors);
    // A reservation within its limit (the values after this write).
    let value = |field: &str, old: Option<i32>| {
        if data.contains_key(field) {
            int(data, field)
        } else {
            old.map(i64::from)
        }
    };
    for (reservation, limit, old_r, old_l) in [
        (
            "cpu_reservation_millis",
            "cpu_limit_millis",
            record.and_then(|r| r.cpu_reservation_millis),
            record.and_then(|r| r.cpu_limit_millis),
        ),
        (
            "mem_reservation_mb",
            "mem_limit_mb",
            record.and_then(|r| r.mem_reservation_mb),
            record.and_then(|r| r.mem_limit_mb),
        ),
    ] {
        if let (Some(r), Some(l)) = (value(reservation, old_r), value(limit, old_l))
            && r > l
        {
            errors.add(reservation, ctx.t("cannot exceed the limit"));
        }
    }
    // A database runs one task (its data volume is its own).
    if let Some(record) = record
        && record.kind != "app"
        && int(data, "replicas").is_some_and(|n| n > 1)
    {
        errors.add("replicas", ctx.t("a database runs one replica"));
    }
    // Two tasks on one data volume: never (a database stops before it starts).
    if let Some(record) = record
        && record.kind != "app"
        && data.get("zero_downtime").and_then(|v| v.as_bool()) == Some(true)
    {
        errors.add(
            "zero_downtime",
            ctx.t("a database stops before it starts again"),
        );
    }
    // Once deployed (or while a first deployment runs), the Docker objects
    // carry the names.
    if let Some(record) = record.filter(|r| r.status != NEW) {
        if text(data, "name").is_some_and(|name| name != record.name) {
            errors.add("name", ctx.t("cannot change once deployed"));
        }
        if int(data, "project_id").is_some_and(|project| project != record.project_id) {
            errors.add("project_id", ctx.t("cannot change once deployed"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}

fn deployment_action(name: &'static str, trigger: Trigger, label: &str) -> Action<service::Entity> {
    Action::<service::Entity>::new(name, move |ctx, txn, services, _| {
        Box::pin(async move {
            let user = ctx.principal.id.parse().ok();
            for service in services {
                let row =
                    deployment_service::request(txn, &service, trigger, None, user, ctx.keyring()?)
                        .await?;
                ctx.dispatch(
                    txn,
                    &RunDeployment {
                        deployment_id: row.id,
                    },
                )
                .await?;
            }
            Ok(())
        })
    })
    .label(label)
    .ability("deploy")
}
