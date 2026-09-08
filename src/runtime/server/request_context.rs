use aio_plugin_identity_server::SessionContext;
use axum::{Json, http::HeaderMap};

use super::{RuntimeState, http_error::RuntimeError};
use crate::runtime::{RuntimeCatalog, RuntimeResponse, UserView};

const MANAGE_PERMISSION: &str = "plugin:manage";

pub(super) async fn authenticate(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<SessionContext, RuntimeError> {
    state
        .identity
        .authenticate(headers)
        .await?
        .ok_or_else(|| RuntimeError::unauthorized("会话无效或已过期"))
}

pub(super) async fn authenticate_manager(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<SessionContext, RuntimeError> {
    let session = authenticate(state, headers).await?;
    if !session
        .permissions
        .iter()
        .any(|permission| permission == MANAGE_PERMISSION)
    {
        return Err(RuntimeError::forbidden("当前角色没有插件管理权限"));
    }
    Ok(session)
}

pub(super) async fn catalog_for(
    state: &RuntimeState,
    session: &SessionContext,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    Ok(Json(RuntimeResponse {
        data: catalog_value(state, session).await?,
    }))
}

pub(super) async fn catalog_value(
    state: &RuntimeState,
    session: &SessionContext,
) -> Result<RuntimeCatalog, RuntimeError> {
    let initials = session
        .display_name
        .chars()
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let mut catalog = state
        .store
        .catalog(
            &session.tenant_id,
            &session.tenant_label,
            UserView {
                label: session.display_name.clone(),
                handle: format!("@{}", session.account),
                initials,
            },
        )
        .await?;
    catalog
        .pages
        .retain(|page| permitted(page.required_permission.as_deref(), &session.permissions));
    catalog
        .account_items
        .retain(|item| permitted(item.required_permission.as_deref(), &session.permissions));
    Ok(catalog)
}

pub(super) fn permitted(required_permission: Option<&str>, permissions: &[String]) -> bool {
    required_permission
        .is_none_or(|permission| permissions.iter().any(|candidate| candidate == permission))
}
