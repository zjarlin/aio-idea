use aio_plugin_identity_server::SessionContext;
use axum::{
    Json,
    http::{HeaderMap, header},
};

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

pub(super) async fn publisher_tenant(
    state: &RuntimeState,
    headers: &HeaderMap,
    git: &str,
    requested_tenant: Option<&str>,
) -> Result<String, RuntimeError> {
    if let Some(token) = bearer_token(headers)
        && let Some(tenant_id) = state.store.publisher_tenant(git, token).await?
    {
        if let Some(requested_tenant) = requested_tenant
            && requested_tenant != tenant_id
        {
            return Err(RuntimeError::forbidden("发布令牌不能切换目标租户"));
        }
        return Ok(tenant_id);
    }
    let session = authenticate_manager(state, headers).await?;
    if let Some(requested_tenant) = requested_tenant
        && requested_tenant != session.tenant_id
    {
        return Err(RuntimeError::forbidden("当前会话不能向其他租户发布插件"));
    }
    Ok(session.tenant_id)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
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
