use aio_plugin_identity_server::SessionContext;
use anyhow::Context as _;
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
use super::lifecycle::{
    cleanup_new_process, cleanup_new_wasm, deactivate_bound_wasm, deactivate_previous_wasm,
    prepare_bound_wasm, prepare_process, prepare_process_revision, prepare_wasm,
    restore_process_binding, stop_previous_process,
};
use crate::runtime::{
    InstallPluginRequest, MarketplaceEntry, PageActionRequest, PageActionResult, PageBody,
    PageDefinition, RuntimeCatalog, RuntimeResponse, UserView,
};

const MANAGE_PERMISSION: &str = "plugin:manage";

pub fn router(state: RuntimeState) -> Router {
    Router::new()
        .route("/api/runtime/catalog", get(catalog))
        .route("/api/runtime/pages/action", post(page_action))
        .route("/api/runtime/marketplace", get(marketplace))
        .route("/api/runtime/registries", post(add_registry))
        .route("/api/runtime/plugins/install", post(install))
        .route(
            "/api/runtime/plugins/{source_id}/events",
            get(lifecycle_events),
        )
        .route("/api/runtime/services/{source_id}/{*path}", any(service))
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

async fn page_action(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<PageActionRequest>,
) -> Result<Json<RuntimeResponse<PageActionResult>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let binding = state
        .store
        .active_page_service(&session.tenant_id, &request.page_id)
        .await?
        .ok_or_else(|| RuntimeError::not_found("当前租户没有提供该页面的活动插件"))?;
    ensure_page_action_allowed(&binding.page, &request.action_id, &session.permissions)?;
    let event = serde_json::json!({
        "kind": "page_action",
        "page_id": request.page_id,
        "action_id": request.action_id,
        "tenant_id": session.tenant_id,
        "user_id": session.user_id,
        "body": &binding.page.body,
    })
    .to_string();
    let result = match binding.service.runtime {
        crate::runtime::PluginRuntime::WasmComponent => {
            let manager = state.wasm.clone();
            let tenant_id = session.tenant_id.clone();
            let source_id = binding.source_id.clone();
            let revision = binding.service.revision.clone();
            let output = tokio::task::spawn_blocking(move || {
                manager.handle(&tenant_id, &source_id, &revision, event)
            })
            .await
            .map_err(|error| {
                RuntimeError::bad_request(format!("等待 Wasm 页面动作失败: {error}"))
            })??;
            if !(200..300).contains(&output.status) {
                return Err(RuntimeError::bad_request(format!(
                    "Wasm 页面动作返回 HTTP {}",
                    output.status
                )));
            }
            serde_json::from_str::<PageActionResult>(&output.body)
                .context("解析 Wasm 页面动作结果失败")?
        }
        crate::runtime::PluginRuntime::Process => {
            let endpoint = binding
                .service
                .endpoint
                .as_deref()
                .context("process 页面插件缺少活动 endpoint")?;
            let output = state
                .process
                .request(
                    endpoint,
                    "POST",
                    "/aio/action",
                    None,
                    event.into_bytes(),
                    Some("application/json"),
                    &session.tenant_id,
                    &session.user_id,
                )
                .await?;
            if !output.status.is_success() {
                return Err(RuntimeError::bad_request(format!(
                    "process 页面动作返回 HTTP {}",
                    output.status
                )));
            }
            serde_json::from_slice::<PageActionResult>(&output.body)
                .context("解析 process 页面动作结果失败")?
        }
        _ => return Err(RuntimeError::bad_request("当前页面运行时不支持动作")),
    };
    validate_page_action_result(&binding.page, &result)?;
    state
        .store
        .save_page_state(
            &session.tenant_id,
            &binding.source_id,
            &binding.revision_id,
            &request.page_id,
            binding.state_generation,
            &result.body,
        )
        .await?;
    Ok(Json(RuntimeResponse { data: result }))
}

fn ensure_page_action_allowed(
    page: &PageDefinition,
    action_id: &str,
    permissions: &[String],
) -> Result<(), RuntimeError> {
    if !permitted(page.required_permission.as_deref(), permissions) {
        return Err(RuntimeError::forbidden("当前角色没有页面动作权限"));
    }
    let PageBody::Actions { actions, .. } = &page.body else {
        return Err(RuntimeError::bad_request("当前页面没有运行时动作"));
    };
    if !actions.iter().any(|action| action.id == action_id) {
        return Err(RuntimeError::bad_request("页面动作未声明"));
    }
    Ok(())
}

fn validate_page_action_result(
    page: &PageDefinition,
    result: &PageActionResult,
) -> Result<(), RuntimeError> {
    let PageBody::Actions {
        actions: declared, ..
    } = &page.body
    else {
        return Err(RuntimeError::bad_request("当前页面没有运行时动作"));
    };
    let PageBody::Actions {
        actions: returned, ..
    } = &result.body
    else {
        return Err(RuntimeError::bad_request(
            "页面动作结果必须保持 actions 页面体",
        ));
    };
    if declared != returned {
        return Err(RuntimeError::bad_request(
            "页面动作结果不能修改已发布的动作声明",
        ));
    }
    let mut updated = page.clone();
    updated.body = result.body.clone();
    az_plugin_manifest::validate_page_definitions(&[updated])?;
    Ok(())
}

async fn install(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<InstallPluginRequest>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let mut discovered = state
        .repository
        .discover(&request.git, request.rev.as_deref())
        .await?;
    if let Some(source_id) = state.store.source_id(&discovered.git).await? {
        discovered.source_id = source_id;
    }
    let current_revision = discovered.revision.clone();
    state
        .store
        .record_lifecycle_event(
            &session.tenant_id,
            &discovered.source_id,
            None,
            "validate",
            "已校验 Git 完整提交、清单和预构建 artifact",
        )
        .await?;
    let previous = state
        .store
        .active_process(&session.tenant_id, &discovered.source_id)
        .await?;
    let previous_target = if previous.is_some() {
        Some(
            state
                .store
                .bound_runtime(&session.tenant_id, &discovered.source_id)
                .await?,
        )
    } else {
        None
    };
    let previous_wasm = state
        .store
        .active_wasm_revision(&session.tenant_id, &discovered.source_id)
        .await?;
    let instance = if discovered.runtime == crate::runtime::PluginRuntime::Process {
        Some(
            match prepare_process(&state, &session.tenant_id, &mut discovered).await {
                Ok(instance) => instance,
                Err(error) => {
                    let _ = state
                        .store
                        .record_lifecycle_event(
                            &session.tenant_id,
                            &discovered.source_id,
                            None,
                            "failed",
                            &format!("process 健康检查失败: {error:#}"),
                        )
                        .await;
                    return Err(error.into());
                }
            },
        )
    } else {
        None
    };
    let wasm = if discovered.runtime == crate::runtime::PluginRuntime::WasmComponent {
        Some(
            match prepare_wasm(&state, &session.tenant_id, &mut discovered).await {
                Ok(activation) => activation,
                Err(error) => {
                    let _ = state
                        .store
                        .record_lifecycle_event(
                            &session.tenant_id,
                            &discovered.source_id,
                            None,
                            "failed",
                            &format!("Wasm Component 健康检查失败: {error:#}"),
                        )
                        .await;
                    return Err(error.into());
                }
            },
        )
    } else {
        None
    };
    if instance.is_some() || wasm.is_some() {
        state
            .store
            .record_lifecycle_event(
                &session.tenant_id,
                &discovered.source_id,
                None,
                "health-check",
                "运行时健康检查通过",
            )
            .await?;
    }
    let source_id = discovered.source_id.clone();
    if let Err(error) = stop_previous_process(&state, previous, instance.as_ref()).await {
        let _ = cleanup_new_process(&state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            &state,
            &session.tenant_id,
            &source_id,
            &current_revision,
            wasm.as_ref(),
        );
        return Err(error.into());
    }
    if let Err(error) = state
        .store
        .activate(&session.tenant_id, discovered, instance.as_ref())
        .await
    {
        let _ = cleanup_new_process(&state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            &state,
            &session.tenant_id,
            &source_id,
            &current_revision,
            wasm.as_ref(),
        );
        if let Err(recovery) = restore_process_binding(
            &state,
            &session.tenant_id,
            &source_id,
            previous_target.as_ref(),
        )
        .await
        {
            return Err(anyhow::anyhow!(
                "激活插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            )
            .into());
        }
        return Err(error.into());
    }
    deactivate_previous_wasm(
        &state,
        &session.tenant_id,
        &source_id,
        previous_wasm.as_deref(),
        wasm.as_ref().map(|_| current_revision.as_str()),
    )?;
    catalog_for(&state, &session).await
}

async fn enable(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let target = state
        .store
        .bound_runtime(&session.tenant_id, &source_id)
        .await?;
    let instance = if target.runtime == crate::runtime::PluginRuntime::Process {
        Some(
            prepare_process_revision(&state, &session.tenant_id, &source_id, &target.revision)
                .await?
                .0,
        )
    } else {
        None
    };
    let wasm = if target.runtime == crate::runtime::PluginRuntime::WasmComponent {
        Some(prepare_bound_wasm(&state, &session.tenant_id, &source_id, &target).await?)
    } else {
        None
    };
    if let Err(error) = state
        .store
        .set_enabled(&session.tenant_id, &source_id, true, instance.as_ref())
        .await
    {
        let _ = cleanup_new_process(&state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            &state,
            &session.tenant_id,
            &source_id,
            &target.revision,
            wasm.as_ref(),
        );
        return Err(error.into());
    }
    catalog_for(&state, &session).await
}

async fn disable(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let previous = state
        .store
        .active_process(&session.tenant_id, &source_id)
        .await?;
    let target = state
        .store
        .bound_runtime(&session.tenant_id, &source_id)
        .await?;
    if let Err(error) = stop_previous_process(&state, previous, None).await {
        return Err(error.into());
    }
    if let Err(error) = state
        .store
        .set_enabled(&session.tenant_id, &source_id, false, None)
        .await
    {
        if let Err(recovery) =
            restore_process_binding(&state, &session.tenant_id, &source_id, Some(&target)).await
        {
            return Err(anyhow::anyhow!(
                "停用插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            )
            .into());
        }
        return Err(error.into());
    }
    deactivate_bound_wasm(&state, &session.tenant_id, &source_id, &target)?;
    catalog_for(&state, &session).await
}

async fn uninstall(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let previous = state
        .store
        .active_process(&session.tenant_id, &source_id)
        .await?;
    let target = state
        .store
        .bound_runtime(&session.tenant_id, &source_id)
        .await?;
    if let Err(error) = stop_previous_process(&state, previous, None).await {
        return Err(error.into());
    }
    if let Err(error) = state.store.uninstall(&session.tenant_id, &source_id).await {
        if let Err(recovery) =
            restore_process_binding(&state, &session.tenant_id, &source_id, Some(&target)).await
        {
            return Err(anyhow::anyhow!(
                "卸载插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            )
            .into());
        }
        return Err(error.into());
    }
    deactivate_bound_wasm(&state, &session.tenant_id, &source_id, &target)?;
    catalog_for(&state, &session).await
}

async fn rollback(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let previous = state
        .store
        .active_process(&session.tenant_id, &source_id)
        .await?;
    let previous_target = if previous.is_some() {
        Some(
            state
                .store
                .bound_runtime(&session.tenant_id, &source_id)
                .await?,
        )
    } else {
        None
    };
    let previous_wasm = state
        .store
        .active_wasm_revision(&session.tenant_id, &source_id)
        .await?;
    let target = state
        .store
        .rollback_target(&session.tenant_id, &source_id)
        .await?;
    let instance = if target.runtime == crate::runtime::PluginRuntime::Process {
        Some(
            prepare_process_revision(&state, &session.tenant_id, &source_id, &target.revision)
                .await?
                .0,
        )
    } else {
        None
    };
    let wasm = if target.runtime == crate::runtime::PluginRuntime::WasmComponent {
        Some(prepare_bound_wasm(&state, &session.tenant_id, &source_id, &target).await?)
    } else {
        None
    };
    if let Err(error) = stop_previous_process(&state, previous, instance.as_ref()).await {
        let _ = cleanup_new_process(&state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            &state,
            &session.tenant_id,
            &source_id,
            &target.revision,
            wasm.as_ref(),
        );
        return Err(error.into());
    }
    if let Err(error) = state
        .store
        .rollback_to(&session.tenant_id, &source_id, &target, instance.as_ref())
        .await
    {
        let _ = cleanup_new_process(&state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            &state,
            &session.tenant_id,
            &source_id,
            &target.revision,
            wasm.as_ref(),
        );
        if let Err(recovery) = restore_process_binding(
            &state,
            &session.tenant_id,
            &source_id,
            previous_target.as_ref(),
        )
        .await
        {
            return Err(anyhow::anyhow!(
                "回滚插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            )
            .into());
        }
        return Err(error.into());
    }
    deactivate_previous_wasm(
        &state,
        &session.tenant_id,
        &source_id,
        previous_wasm.as_deref(),
        wasm.as_ref().map(|_| target.revision.as_str()),
    )?;
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
    for plugin in &catalog.plugins {
        if !entries
            .iter()
            .any(|entry: &MarketplaceEntry| entry.git == plugin.git)
        {
            entries.push(unlisted_entry(plugin));
        }
    }
    Ok(Json(RuntimeResponse { data: entries }))
}

async fn lifecycle_events(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<Vec<crate::runtime::PluginLifecycleEvent>>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let events = state
        .store
        .lifecycle_events(&session.tenant_id, &source_id)
        .await?;
    Ok(Json(RuntimeResponse { data: events }))
}

fn unlisted_entry(plugin: &crate::runtime::InstalledPluginView) -> MarketplaceEntry {
    let title = plugin
        .git
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("Git plugin")
        .to_owned();
    MarketplaceEntry {
        git: plugin.git.clone(),
        rev: plugin.revision.clone(),
        title,
        summary: "当前租户直接安装的未收录 Git 插件。".to_owned(),
        license: "未收录".to_owned(),
        tags: vec!["unlisted".to_owned()],
        installed: true,
        source_id: Some(plugin.source_id.clone()),
        state: Some(plugin.state),
        active_revision: Some(plugin.revision.clone()),
        runtime: Some(plugin.runtime),
    }
}

async fn service(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path((source_id, path)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    body: Bytes,
) -> Result<Response, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let binding = state
        .store
        .active_service(&session.tenant_id, &source_id)
        .await?
        .ok_or_else(|| RuntimeError::not_found("当前租户没有活动的服务插件"))?;
    ensure_route_allowed(&binding.routes, &path)?;
    let path = format!("/{path}");
    match binding.runtime {
        crate::runtime::PluginRuntime::WasmComponent => {
            let body = String::from_utf8(body.to_vec())
                .map_err(|_| RuntimeError::bad_request("Wasm Component 请求体必须是 UTF-8"))?;
            let request = serde_json::json!({
                "method": method.as_str(),
                "path": path,
                "query": uri.query(),
                "body": body,
                "tenant_id": session.tenant_id,
                "user_id": session.user_id,
            })
            .to_string();
            let manager = state.wasm.clone();
            let tenant_id = session.tenant_id.clone();
            let revision = binding.revision.clone();
            let output = tokio::task::spawn_blocking(move || {
                manager.handle(&tenant_id, &source_id, &revision, request)
            })
            .await
            .map_err(|error| {
                RuntimeError::bad_request(format!("等待 Wasm 请求处理失败: {error}"))
            })??;
            let status = StatusCode::from_u16(output.status)
                .map_err(|_| RuntimeError::bad_request("Wasm Component 返回了无效状态码"))?;
            response(status, Some(&output.content_type), output.body.into_bytes())
        }
        crate::runtime::PluginRuntime::Process => {
            let endpoint = binding
                .endpoint
                .as_deref()
                .context("process 插件缺少活动 endpoint")?;
            let content_type = headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            let output = state
                .process
                .request(
                    endpoint,
                    method.as_str(),
                    &path,
                    uri.query(),
                    body.to_vec(),
                    content_type,
                    &session.tenant_id,
                    &session.user_id,
                )
                .await?;
            response(output.status, output.content_type.as_deref(), output.body)
        }
        _ => Err(RuntimeError::not_found("当前插件不提供动态服务")),
    }
}

fn ensure_route_allowed(routes: &[String], path: &str) -> Result<(), RuntimeError> {
    if routes
        .iter()
        .any(|route| path == route || path.starts_with(&format!("{route}/")))
    {
        return Ok(());
    }
    Err(RuntimeError::not_found("插件清单未声明该服务路由"))
}

fn response(
    status: StatusCode,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Result<Response, RuntimeError> {
    let mut response = (status, body).into_response();
    if let Some(content_type) = content_type {
        let content_type = HeaderValue::from_str(content_type)
            .map_err(|_| RuntimeError::bad_request("插件返回了无效 Content-Type"))?;
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type);
    }
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
    catalog
        .pages
        .retain(|page| permitted(page.required_permission.as_deref(), &session.permissions));
    catalog
        .account_items
        .retain(|item| permitted(item.required_permission.as_deref(), &session.permissions));
    Ok(catalog)
}

fn permitted(required_permission: Option<&str>, permissions: &[String]) -> bool {
    required_permission
        .is_none_or(|permission| permissions.iter().any(|candidate| candidate == permission))
}

pub(super) struct RuntimeError {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{InstalledPluginView, PluginRuntime, PluginState};
    use az_plugin_manifest::{PageActionDefinition, SceneDefinition};

    fn action_page(required_permission: Option<&str>) -> PageDefinition {
        PageDefinition {
            id: "counter".to_owned(),
            label: "Counter".to_owned(),
            icon: None,
            scene: SceneDefinition {
                id: "examples".to_owned(),
                label: "Examples".to_owned(),
            },
            required_permission: required_permission.map(str::to_owned),
            body: PageBody::Actions {
                title: "Counter".to_owned(),
                content: "0".to_owned(),
                state: [("count".to_owned(), serde_json::json!(0))]
                    .into_iter()
                    .collect(),
                actions: vec![PageActionDefinition {
                    id: "increment".to_owned(),
                    label: "+1".to_owned(),
                }],
            },
        }
    }

    #[test]
    fn builds_manageable_entry_for_unlisted_plugin() {
        let entry = unlisted_entry(&InstalledPluginView {
            source_id: "source".to_owned(),
            git: "https://github.com/example/aio-plugin-kmp.git".to_owned(),
            revision: "0".repeat(40),
            runtime: PluginRuntime::PageDefinition,
            state: PluginState::Active,
        });

        assert_eq!(entry.title, "aio-plugin-kmp");
        assert!(entry.installed);
        assert_eq!(entry.source_id.as_deref(), Some("source"));
        assert_eq!(entry.tags, vec!["unlisted"]);
    }

    #[test]
    fn allows_only_declared_route_prefixes() {
        let routes = vec!["echo".to_owned(), "jobs/status".to_owned()];
        assert!(ensure_route_allowed(&routes, "echo").is_ok());
        assert!(ensure_route_allowed(&routes, "echo/detail").is_ok());
        assert!(ensure_route_allowed(&routes, "jobs/status/current").is_ok());
        assert!(ensure_route_allowed(&routes, "jobs").is_err());
        assert!(ensure_route_allowed(&routes, "other").is_err());
    }

    #[test]
    fn permission_gate_rejects_ungranted_account_contributions() {
        let permissions = vec!["workspace:view".to_owned()];

        assert!(permitted(None, &permissions));
        assert!(permitted(Some("workspace:view"), &permissions));
        assert!(!permitted(Some("plugin:manage"), &permissions));
    }

    #[test]
    fn allows_only_declared_page_actions_with_permission() {
        let page = action_page(Some("counter:use"));
        let permissions = vec!["counter:use".to_owned()];

        assert!(ensure_page_action_allowed(&page, "increment", &permissions).is_ok());
        assert!(ensure_page_action_allowed(&page, "missing", &permissions).is_err());
        assert!(ensure_page_action_allowed(&page, "increment", &[]).is_err());
    }

    #[test]
    fn action_result_cannot_change_published_actions() {
        let page = action_page(None);
        let valid = PageActionResult {
            body: PageBody::Actions {
                title: "Counter".to_owned(),
                content: "1".to_owned(),
                state: [("count".to_owned(), serde_json::json!(1))]
                    .into_iter()
                    .collect(),
                actions: vec![PageActionDefinition {
                    id: "increment".to_owned(),
                    label: "+1".to_owned(),
                }],
            },
        };
        assert!(validate_page_action_result(&page, &valid).is_ok());

        let changed = PageActionResult {
            body: PageBody::Actions {
                title: "Counter".to_owned(),
                content: "1".to_owned(),
                state: [("count".to_owned(), serde_json::json!(1))]
                    .into_iter()
                    .collect(),
                actions: vec![PageActionDefinition {
                    id: "reset".to_owned(),
                    label: "Reset".to_owned(),
                }],
            },
        };
        assert!(validate_page_action_result(&page, &changed).is_err());
        assert!(
            validate_page_action_result(
                &page,
                &PageActionResult {
                    body: PageBody::Text {
                        title: "Counter".to_owned(),
                        content: "1".to_owned(),
                    },
                },
            )
            .is_err()
        );
    }
}
