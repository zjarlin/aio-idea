use anyhow::Context as _;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post},
};
use serde::Deserialize;

use super::{
    RuntimeState,
    http_error::RuntimeError,
    lifecycle::{
        cleanup_new_process, cleanup_new_wasm, deactivate_bound_wasm, deactivate_previous_wasm,
        prepare_bound_wasm, prepare_process_revision, restore_process_binding,
        stop_previous_process,
    },
    marketplace_store::PUBLISHED_REGISTRY_SOURCE,
    repository::validate_git,
    request_context::{
        authenticate, authenticate_manager, catalog_for, catalog_value, permitted, publisher_tenant,
    },
};
use crate::runtime::{
    CreatePublishCredentialRequest, InstallPluginRequest, MarketplaceEntry, PageActionRequest,
    PageActionResult, PageBody, PageDefinition, PluginRequest, PublishCredentialView,
    PublishPluginRequest, PublishedPluginView, RuntimeCatalog, RuntimeResponse,
};

pub fn router(state: RuntimeState) -> Router {
    Router::new()
        .route("/api/runtime/catalog", get(catalog))
        .route("/api/runtime/pages/action", post(page_action))
        .route("/api/runtime/marketplace", get(marketplace))
        .route("/api/runtime/registries", post(add_registry))
        .route("/api/runtime/plugins/install", post(install))
        .route("/api/runtime/plugins/publish", post(publish))
        .route(
            "/api/runtime/publish-credentials",
            post(create_publish_credential),
        )
        .route(
            "/api/runtime/publish-credentials/{credential_id}",
            delete(revoke_publish_credential),
        )
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
    let event = serde_json::to_string(&PluginRequest::PageAction {
        page_id: request.page_id.clone(),
        action_id: request.action_id.clone(),
        tenant_id: session.tenant_id.clone(),
        user_id: session.user_id.clone(),
        body: binding.page.body.clone(),
    })
    .context("序列化页面动作请求失败")?;
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
    let discovered = state
        .repository
        .discover(&request.git, request.rev.as_deref())
        .await?;
    super::installation::activate(
        &state,
        &session.tenant_id,
        discovered,
        "已校验 Git 完整提交、清单和预构建 artifact",
    )
    .await?;
    catalog_for(&state, &session).await
}

async fn publish(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<PublishPluginRequest>,
) -> Result<Json<RuntimeResponse<PublishedPluginView>>, RuntimeError> {
    let tenant_id =
        publisher_tenant(&state, &headers, &request.git, request.tenant_id.as_deref()).await?;
    let publication = request
        .marketplace
        .as_ref()
        .map(|metadata| {
            let manifest = az_plugin_manifest::parse_manifest(&request.manifest_toml)?;
            Ok::<_, anyhow::Error>(MarketplaceEntry {
                git: request.git.clone(),
                rev: request.rev.clone(),
                title: metadata.title.clone(),
                summary: metadata.summary.clone(),
                license: metadata.license.clone(),
                tags: metadata.tags.clone(),
                installed: false,
                source_id: None,
                state: None,
                active_revision: None,
                runtime: manifest.plugin.runtime.map(|runtime| runtime.kind),
                capabilities: manifest.plugin.capabilities,
            })
        })
        .transpose()?;
    let discovered = state.repository.publish(&request).await?;
    let activated = super::installation::activate(
        &state,
        &tenant_id,
        discovered,
        "已校验 CI 发布的清单、artifact SHA-256 和运行时协议",
    )
    .await?;
    if let Some(entry) = publication {
        state
            .store
            .upsert_published_marketplace_entry(&entry)
            .await?;
    }
    Ok(Json(RuntimeResponse {
        data: PublishedPluginView {
            tenant_id,
            source_id: activated.source_id,
            revision: activated.revision,
            runtime: activated.runtime,
            page_count: activated.page_count,
        },
    }))
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
    let mut remote_sources = vec![state.marketplace_url.clone()];
    remote_sources.extend(state.store.registry_sources(&session.tenant_id).await?);
    state.sync_marketplace_sources(remote_sources.clone());
    let mut sources = vec![PUBLISHED_REGISTRY_SOURCE.to_owned()];
    sources.extend(remote_sources);
    let mut entries = Vec::new();
    for source in sources {
        for mut entry in state.store.marketplace_entries(&source).await? {
            if let Some(plugin) = catalog
                .plugins
                .iter()
                .find(|plugin| plugin.git == entry.git)
            {
                enrich_marketplace_entry(&mut entry, plugin);
            }
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

fn enrich_marketplace_entry(
    entry: &mut MarketplaceEntry,
    plugin: &crate::runtime::InstalledPluginView,
) {
    entry.source_id = Some(plugin.source_id.clone());
    entry.installed = true;
    entry.state = Some(plugin.state);
    entry.active_revision = Some(plugin.revision.clone());
    entry.runtime = Some(plugin.runtime);
    entry.capabilities.clone_from(&plugin.capabilities);
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
        capabilities: plugin.capabilities.clone(),
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
            let request = serde_json::to_string(&PluginRequest::ServiceRequest {
                method: method.as_str().to_owned(),
                path,
                query: uri.query().map(str::to_owned),
                body,
                tenant_id: session.tenant_id.clone(),
                user_id: session.user_id.clone(),
            })
            .context("序列化 Wasm 服务请求失败")?;
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

async fn create_publish_credential(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<CreatePublishCredentialRequest>,
) -> Result<Json<RuntimeResponse<PublishCredentialView>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    validate_git(&request.git)?;
    let credential = state
        .store
        .create_publish_credential(&session.tenant_id, &request.git)
        .await?;
    Ok(Json(RuntimeResponse { data: credential }))
}

async fn revoke_publish_credential(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(credential_id): Path<String>,
) -> Result<StatusCode, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    state
        .store
        .revoke_publish_credential(&session.tenant_id, &credential_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[path = "routes_tests.rs"]
mod tests;
