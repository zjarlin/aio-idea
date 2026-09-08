use aio_plugin_identity_server::SessionContext;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use serde::Deserialize;

use super::RuntimeState;
use crate::runtime::{
    InstallPluginRequest, MarketplaceEntry, RuntimeCatalog, RuntimeResponse, UserView,
};

const MANAGE_PERMISSION: &str = "plugin:manage";

pub fn router(state: RuntimeState) -> Router {
    Router::new()
        .route("/api/runtime/catalog", get(catalog))
        .route("/api/runtime/marketplace", get(marketplace))
        .route("/api/runtime/registries", post(add_registry))
        .route("/api/runtime/plugins/install", post(install))
        .route(
            "/api/runtime/components/{source_id}/{*path}",
            any(component),
        )
        .route("/api/runtime/plugins/{source_id}/enable", post(enable))
        .route("/api/runtime/plugins/{source_id}/disable", post(disable))
        .route("/api/runtime/plugins/{source_id}/rollback", post(rollback))
        .route(
            "/api/runtime/plugins/{source_id}/uninstall",
            post(uninstall),
        )
        .with_state(state)
}

async fn catalog(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    catalog_for(&state, &session).await
}

async fn install(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<InstallPluginRequest>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let discovered = state
        .repository
        .discover(&request.git, request.rev.as_deref())
        .await?;
    state.store.activate(&session.tenant_id, discovered).await?;
    catalog_for(&state, &session).await
}

async fn enable(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    state
        .store
        .set_enabled(&session.tenant_id, &source_id, true)
        .await?;
    catalog_for(&state, &session).await
}

async fn disable(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    state
        .store
        .set_enabled(&session.tenant_id, &source_id, false)
        .await?;
    catalog_for(&state, &session).await
}

async fn uninstall(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    state
        .store
        .uninstall(&session.tenant_id, &source_id)
        .await?;
    catalog_for(&state, &session).await
}

async fn rollback(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    state.store.rollback(&session.tenant_id, &source_id).await?;
    catalog_for(&state, &session).await
}

async fn marketplace(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<Vec<MarketplaceEntry>>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let catalog = catalog_value(&state, &session).await?;
    let mut sources = vec![state.marketplace_url.clone()];
    sources.extend(state.store.registry_sources(&session.tenant_id).await?);
    let mut entries = Vec::new();
    for source in sources {
        for mut entry in state.repository.registry(&source).await? {
            entry.source_id = catalog
                .plugins
                .iter()
                .find(|plugin| plugin.git == entry.git)
                .map(|plugin| plugin.source_id.clone());
            entry.installed = entry.source_id.is_some();
            entry.state = catalog
                .plugins
                .iter()
                .find(|plugin| plugin.git == entry.git)
                .map(|plugin| plugin.state);
            entry.active_revision = catalog
                .plugins
                .iter()
                .find(|plugin| plugin.git == entry.git)
                .map(|plugin| plugin.revision.clone());
            entry.runtime = catalog
                .plugins
                .iter()
                .find(|plugin| plugin.git == entry.git)
                .map(|plugin| plugin.runtime);
            if !entries
                .iter()
                .any(|current: &MarketplaceEntry| current.git == entry.git)
            {
                entries.push(entry);
            }
        }
    }
    Ok(Json(RuntimeResponse { data: entries }))
}

async fn component(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path((source_id, path)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    body: Bytes,
) -> Result<Response, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let (revision, artifact) = state
        .store
        .active_component(&session.tenant_id, &source_id)
        .await?
        .ok_or_else(|| RuntimeError::not_found("当前租户没有活动的 Wasm Component"))?;
    let artifact = state.repository.artifact(&revision, &artifact)?;
    let body = String::from_utf8(body.to_vec())
        .map_err(|_| RuntimeError::bad_request("Wasm Component 请求体必须是 UTF-8"))?;
    let request = serde_json::json!({
        "method": method.as_str(),
        "path": format!("/{path}"),
        "query": uri.query(),
        "body": body,
        "tenant_id": session.tenant_id,
        "user_id": session.user_id,
    })
    .to_string();
    let output = tokio::task::spawn_blocking(move || super::wasm::handle(&artifact, request))
        .await
        .map_err(|error| RuntimeError::bad_request(format!("等待 Wasm 请求处理失败: {error}")))??;
    let status = StatusCode::from_u16(output.status)
        .map_err(|_| RuntimeError::bad_request("Wasm Component 返回了无效状态码"))?;
    let content_type = HeaderValue::from_str(&output.content_type)
        .map_err(|_| RuntimeError::bad_request("Wasm Component 返回了无效 Content-Type"))?;
    let mut response = (status, output.body).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    Ok(response)
}

#[derive(Deserialize)]
struct RegistryRequest {
    source: String,
}

async fn add_registry(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<RegistryRequest>,
) -> Result<StatusCode, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let source = request.source.trim();
    if !source.starts_with("https://") {
        return Err(RuntimeError::bad_request("市场来源必须使用 HTTPS"));
    }
    state.store.add_registry(&session.tenant_id, source).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn authenticate(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<SessionContext, RuntimeError> {
    state
        .identity
        .authenticate(headers)
        .await?
        .ok_or_else(|| RuntimeError::unauthorized("会话无效或已过期"))
}

async fn authenticate_manager(
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

async fn catalog_for(
    state: &RuntimeState,
    session: &SessionContext,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    Ok(Json(RuntimeResponse {
        data: catalog_value(state, session).await?,
    }))
}

async fn catalog_value(
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
    catalog.pages.retain(|page| {
        page.required_permission
            .as_deref()
            .is_none_or(|permission| {
                session
                    .permissions
                    .iter()
                    .any(|candidate| candidate == permission)
            })
    });
    Ok(catalog)
}

struct RuntimeError {
    status: StatusCode,
    error: anyhow::Error,
}

impl RuntimeError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: anyhow::anyhow!(message.into()),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: anyhow::anyhow!(message.into()),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            error: anyhow::anyhow!(message.into()),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error: anyhow::anyhow!(message.into()),
        }
    }
}

impl<E> From<E> for RuntimeError
where
    E: Into<anyhow::Error>,
{
    fn from(value: E) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: value.into(),
        }
    }
}

impl IntoResponse for RuntimeError {
    fn into_response(self) -> Response {
        let message = format!("{:#}", self.error);
        (self.status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
