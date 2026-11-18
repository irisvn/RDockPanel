//! Blocked addresses, the WAF's and the firewall's together (the WAF page's
//! IP lists, the Firewall page's blocked addresses, an alert's menu): the
//! rules are `ip_block_service`'s. The panel's administrators'.

use rustcms::audit::RequestMeta;
use rustcms::prelude::*;
use rustcms::{AppResult, Json};
use serde_json::Value;

use crate::services::action_service::Initiator;
use crate::services::deploy_rules_service::Install;
use crate::services::ip_block_service::{self, Removal, Request};

fn installed() -> Install {
    Install::current()
}

/// `GET /blocks`: every blocked address, where, until when; what each
/// layer can do here; the caller's own address (never blocked).
pub async fn index(State(app): State<AppState>, request: RequestMeta) -> AppResult<Json<Value>> {
    Ok(Json(
        ip_block_service::list(&app, request.ip.as_deref(), installed()).await?,
    ))
}

/// `POST /blocks`: blocks an address on the websites, the server or both.
pub async fn store(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Request>,
) -> AppResult<Json<Value>> {
    let by = Initiator::user(auth.principal(), request);
    Ok(Json(
        ip_block_service::block(&app, &by, &body, installed()).await?,
    ))
}

/// `POST /blocks/remove`: unblocks it where asked.
pub async fn remove(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Removal>,
) -> AppResult<Json<Value>> {
    let by = Initiator::user(auth.principal(), request);
    Ok(Json(
        ip_block_service::unblock(&app, &by, &body, installed()).await?,
    ))
}
