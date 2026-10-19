//! The panel's own alerts, checked on a schedule and said to the channels
//! that hear them (`notify_service`): a running service that fails and
//! comes back, Traefik down and up, a newer panel image, a sign-in from a
//! new address. What was said is kept (`panel_settings`): said once, even
//! across restarts, and again only once it changed.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rustcms::{AppResult, AppState};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::models::{panel_setting, project, service};
use crate::services::docker_service::{Docker, SwarmTask, TaskState};
use crate::services::notify_service::{Event, notify};

/// What is down now (`service:<id>`, `traefik`), as last said.
const STATE_KEY: &str = "alerts.down";
/// The newest panel image said.
const UPDATE_KEY: &str = "alerts.update";
/// The sign-ins read up to.
const LOGIN_KEY: &str = "alerts.logins";
/// This many failed tasks lately: a service that keeps restarting.
const RESTARTS: usize = 3;
/// How far back "lately" goes.
const LATELY: chrono::TimeDelta = chrono::TimeDelta::minutes(10);

async fn down_now<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<BTreeMap<String, String>> {
    Ok(panel_setting::get(db, STATE_KEY)
        .await?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
}

/// Whether `tasks` say the service is failing — fewer than `replicas` run,
/// or they keep failing since it was deployed (`since`: older failures were
/// another rollout's) — and the newest error.
pub fn failing(
    tasks: &[SwarmTask],
    replicas: usize,
    since: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> (bool, Option<String>) {
    let running = tasks
        .iter()
        .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
        .filter(|t| t.desired_state == Some(TaskState::RUNNING))
        .count();
    let mut failed: Vec<&SwarmTask> = tasks
        .iter()
        .filter(|t| {
            t.status.as_ref().is_some_and(|s| {
                matches!(s.state, Some(TaskState::FAILED | TaskState::REJECTED))
                    && s.timestamp.is_some_and(|at| {
                        now - at <= LATELY && since.is_none_or(|since| at >= since)
                    })
            })
        })
        .collect();
    failed.sort_by_key(|t| std::cmp::Reverse(t.status.as_ref().and_then(|s| s.timestamp)));
    let error = failed
        .first()
        .and_then(|t| t.status.as_ref())
        .and_then(|s| s.err.clone().or_else(|| s.message.clone()));
    (running < replicas.max(1) || failed.len() >= RESTARTS, error)
}

/// Running services (deployed, not deploying now, not Compose) and Traefik:
/// a change since last said is said.
pub async fn check_health(state: &AppState) -> AppResult<()> {
    let Some(docker) = state.service::<Docker>() else {
        return Ok(());
    };
    let db = state.db();
    let mut down = down_now(db).await?;
    let before = down.clone();
    let now = Utc::now();
    let projects: BTreeMap<i64, String> = project::Entity::find()
        .all(db)
        .await?
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    let services = service::Entity::find().all(db).await?;
    let mut seen = std::collections::BTreeSet::new();
    for row in &services {
        let key = format!("service:{}", row.id);
        seen.insert(key.clone());
        // A deployment (or a stop) changes its tasks: judged once it ends.
        if row.kind == "compose"
            || row.desired_state != "running"
            || row.status != "running"
            || row.current_deployment_id.is_some()
            || row.replicas == 0
        {
            continue;
        }
        let Some(project) = projects.get(&row.project_id) else {
            continue;
        };
        let name = crate::services::deploy_service::service_name(project, &row.name);
        let Ok(tasks) = docker.engine().list_tasks(&name).await else {
            continue;
        };
        let (failing, error) = failing(
            &tasks,
            usize::try_from(row.replicas).unwrap_or(1),
            row.deployed_at,
            now,
        );
        // Still as read (no deployment began meanwhile: it replaces tasks).
        let again = service::Entity::find_by_id(row.id).one(db).await?;
        if again.is_none_or(|r| {
            r.current_deployment_id.is_some()
                || r.desired_state != "running"
                || r.deployed_at != row.deployed_at
        }) {
            continue;
        }
        let was = down.contains_key(&key);
        // Failing at two checks in a row (a task being replaced is not).
        let suspect = format!("suspect:{}", row.id);
        if failing && !was && !down.contains_key(&suspect) {
            down.insert(suspect, now.to_rfc3339());
            continue;
        }
        down.remove(&suspect);
        if failing && !was {
            down.insert(key, now.to_rfc3339());
            notify(
                state,
                Event::Service {
                    project: project.clone(),
                    service: row.name.clone(),
                    down: true,
                    error,
                },
            );
        } else if !failing && was {
            down.remove(&key);
            notify(
                state,
                Event::Service {
                    project: project.clone(),
                    service: row.name.clone(),
                    down: false,
                    error: None,
                },
            );
        }
    }
    // Services gone (destroyed, deleted) are forgotten.
    down.retain(|k, _| {
        let service = k
            .strip_prefix("suspect:")
            .map(|id| format!("service:{id}"))
            .unwrap_or_else(|| k.clone());
        !service.starts_with("service:") || seen.contains(&service)
    });
    // Installed, Traefik is the installer's service: gone counts as down.
    // From source, only one that exists is watched.
    let traefik = &crate::services::deploy_rules_service::config().traefik_service;
    let found = match docker.engine().inspect_service(traefik).await {
        Ok(found) if !traefik.is_empty() => Some(found.is_some()),
        _ => None,
    };
    if let Some(exists) = found
        && (exists || crate::services::deploy_rules_service::config().installed)
    {
        let tasks = if exists {
            docker
                .engine()
                .list_tasks(traefik)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };
        let running = tasks.iter().any(|t| {
            t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING)
                && t.desired_state == Some(TaskState::RUNNING)
        });
        let was = down.contains_key("traefik");
        if !running && !was {
            down.insert("traefik".to_owned(), now.to_rfc3339());
            notify(state, Event::Traefik { down: true });
        } else if running && was {
            down.remove("traefik");
            notify(state, Event::Traefik { down: false });
        }
    }
    if down != before {
        panel_setting::set(
            db,
            STATE_KEY,
            &serde_json::to_string(&down).map_err(rustcms::AppError::internal)?,
        )
        .await?;
    }
    Ok(())
}

/// A newer panel image (installed only): said once per image.
pub async fn check_update(state: &AppState) -> AppResult<()> {
    let cfg = crate::services::deploy_rules_service::config();
    let Some(docker) = state.service::<Docker>() else {
        return Ok(());
    };
    if !cfg.installed {
        return Ok(());
    }
    let found = crate::services::system_service::check_update(&docker, true).await?;
    let (Some(true), Some(latest), Some(image)) = (
        found["update"].as_bool(),
        found["latest"].as_str(),
        found["image"].as_str(),
    ) else {
        return Ok(());
    };
    if panel_setting::get(state.db(), UPDATE_KEY).await?.as_deref() == Some(latest) {
        return Ok(());
    }
    panel_setting::set(state.db(), UPDATE_KEY, latest).await?;
    notify(
        state,
        Event::Update {
            image: image.to_owned(),
        },
    );
    Ok(())
}

/// Sign-ins since the last read (Core's audit, `auth.signed_in`, in the
/// order of their ids: UUID v7, by time): one from an address its user
/// never signed in from before is said. The first read only starts there
/// (nothing earlier is said). The address is the client's as the trusted
/// proxies let the panel see it (`http.trusted_proxies`; the installer
/// trusts the private ranges Traefik reaches the panel from).
pub async fn check_logins(state: &AppState) -> AppResult<()> {
    use rustcms::models::audit_log;
    let db = state.db();
    let signed_in =
        || audit_log::Entity::find().filter(audit_log::Column::Event.eq("auth.signed_in"));
    let since = panel_setting::get(db, LOGIN_KEY)
        .await?
        .and_then(|v| v.parse::<sea_orm::prelude::Uuid>().ok());
    let Some(since) = since else {
        let last = signed_in()
            .order_by_desc(audit_log::Column::Id)
            .one(db)
            .await?
            .map(|r| r.id)
            .unwrap_or_default();
        panel_setting::set(db, LOGIN_KEY, &last.to_string()).await?;
        return Ok(());
    };
    let rows = signed_in()
        .filter(audit_log::Column::Id.gt(since))
        .order_by_asc(audit_log::Column::Id)
        .limit(200)
        .all(db)
        .await?;
    let mut last = since;
    for row in rows {
        last = row.id;
        let (Some(user), Some(ip)) = (row.actor_user_id, row.ip.clone()) else {
            continue;
        };
        let earlier = signed_in()
            .filter(audit_log::Column::ActorUserId.eq(user))
            .filter(audit_log::Column::Id.lt(row.id));
        // A first sign-in ever is no new address.
        if earlier.clone().count(db).await? == 0
            || earlier
                .filter(audit_log::Column::Ip.eq(ip.clone()))
                .count(db)
                .await?
                > 0
        {
            continue;
        }
        let name = rustcms::models::user::Entity::find_by_id(user)
            .one(db)
            .await?
            .map(|u| u.email)
            .unwrap_or_else(|| format!("user {user}"));
        notify(state, Event::Login { user: name, ip });
    }
    if last != since {
        panel_setting::set(db, LOGIN_KEY, &last.to_string()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::docker_service::TaskStatus;

    fn task(state: TaskState, ago: i64) -> SwarmTask {
        SwarmTask {
            desired_state: Some(TaskState::RUNNING),
            status: Some(TaskStatus {
                state: Some(state),
                timestamp: Some(Utc::now() - chrono::TimeDelta::minutes(ago)),
                err: (state == TaskState::FAILED).then(|| format!("exit {ago}")),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn failing_services() {
        let now = Utc::now();
        let check = |tasks: &[SwarmTask], replicas, since| failing(tasks, replicas, since, now);
        assert_eq!(
            check(&[task(TaskState::RUNNING, 0)], 1, None),
            (false, None)
        );
        // One failure long ago, running now: fine.
        let old = [task(TaskState::RUNNING, 0), task(TaskState::FAILED, 30)];
        assert!(!check(&old, 1, None).0);
        // One of two replicas runs.
        assert!(check(&[task(TaskState::RUNNING, 0)], 2, None).0);
        // Restarting: three failures lately, though one runs.
        let restarting = [
            task(TaskState::RUNNING, 0),
            task(TaskState::FAILED, 1),
            task(TaskState::FAILED, 2),
            task(TaskState::FAILED, 3),
        ];
        let (down, error) = check(&restarting, 1, None);
        assert!(down);
        assert_eq!(error.as_deref(), Some("exit 1"));
        // Those failures before its last deployment: another rollout's.
        assert!(!check(&restarting, 1, Some(now - chrono::TimeDelta::seconds(30))).0);
        // None runs.
        assert!(check(&[task(TaskState::SHUTDOWN, 1)], 1, None).0);
    }
}
