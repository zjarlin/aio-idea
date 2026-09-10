use std::env;

use aio_plugin_identity_server::SessionContext;
use axum::{
    Json,
    http::{HeaderMap, header},
};

use super::{RuntimeState, http_error::RuntimeError};
use crate::runtime::{RuntimeCatalog, RuntimeResponse, UserView};

const MANAGE_PERMISSION: &str = "plugin:manage";
const PUBLISH_ACCOUNTS_ENV: &str = "AIO_PLUGIN_PUBLISH_ACCOUNTS";

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

pub(super) async fn authenticate_publish_manager(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<SessionContext, RuntimeError> {
    let session = authenticate_manager(state, headers).await?;
    let configured = env::var(PUBLISH_ACCOUNTS_ENV).ok();
    if !publish_account_allowed(configured.as_deref(), &session.account) {
        return Err(RuntimeError::forbidden("当前账号没有平台插件发布权限"));
    }
    Ok(session)
}

pub(super) struct PublisherContext {
    pub tenant_id: String,
    pub git: Option<String>,
}

pub(super) async fn authenticate_publisher(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<PublisherContext, RuntimeError> {
    if let Some(token) = bearer_token(headers) {
        let binding = state
            .store
            .publisher_binding(token)
            .await?
            .ok_or_else(|| RuntimeError::unauthorized("发布令牌无效或已撤销"))?;
        return Ok(PublisherContext {
            tenant_id: binding.tenant_id,
            git: Some(binding.git),
        });
    }
    let session = authenticate_publish_manager(state, headers).await?;
    Ok(PublisherContext {
        tenant_id: session.tenant_id,
        git: None,
    })
}

pub(super) fn authorize_publish_target(
    publisher: PublisherContext,
    git: &str,
    requested_tenant: Option<&str>,
) -> Result<String, RuntimeError> {
    if let Some(credential_git) = publisher.git
        && credential_git != git
    {
        return Err(RuntimeError::forbidden("发布令牌不能更新其他 Git 来源"));
    }
    if let Some(requested_tenant) = requested_tenant
        && requested_tenant != publisher.tenant_id
    {
        return Err(RuntimeError::forbidden("当前会话不能向其他租户发布插件"));
    }
    Ok(publisher.tenant_id)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn publish_account_allowed(configured: Option<&str>, account: &str) -> bool {
    configured.is_some_and(|configured| {
        configured
            .split(',')
            .map(str::trim)
            .any(|candidate| !candidate.is_empty() && candidate == account)
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_credential_is_bound_to_git_and_tenant() {
        let publisher = PublisherContext {
            tenant_id: "tenant-a".to_owned(),
            git: Some("https://github.com/example/plugin.git".to_owned()),
        };
        assert!(
            authorize_publish_target(
                publisher,
                "https://github.com/example/other.git",
                Some("tenant-a"),
            )
            .is_err()
        );

        let publisher = PublisherContext {
            tenant_id: "tenant-a".to_owned(),
            git: Some("https://github.com/example/plugin.git".to_owned()),
        };
        assert!(
            authorize_publish_target(
                publisher,
                "https://github.com/example/plugin.git",
                Some("tenant-b"),
            )
            .is_err()
        );
    }

    #[test]
    fn platform_publish_accounts_are_explicit_and_exact() {
        assert!(!publish_account_allowed(None, "zjarlin"));
        assert!(publish_account_allowed(Some("alice, zjarlin"), "zjarlin"));
        assert!(!publish_account_allowed(
            Some("alice,zjarlin-admin"),
            "zjarlin"
        ));
    }
}
