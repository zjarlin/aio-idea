use super::super::{
    RuntimeState,
    frontend_access::FrontendGrant,
    frontend_model::MountResponse,
    http_error::RuntimeError,
    request_context::{authenticate, permitted, session_context, tenant_context},
};
use super::model;
use crate::runtime::RuntimeResponse;
use aio_plugin_identity_server::SessionContext;
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use kuchikiki::traits::TendrilSink;
use std::time::Instant;
use uuid::Uuid;

fn identity(page: &str) -> Result<(Uuid, &str)> {
    let (source, page) = page
        .strip_prefix("component:")
        .and_then(|id| id.split_once(':'))
        .context("Component 页面 ID 无效")?;
    Ok((Uuid::parse_str(source)?, page))
}

async fn page(
    state: &RuntimeState,
    session: &SessionContext,
    id: &str,
) -> Result<(Uuid, String, String, model::Page)> {
    let (source, id) = identity(id)?;
    let (digest, generation, description) = state
        .components()?
        .description(&session.tenant_id, source)
        .await?;
    let page = description
        .pages
        .into_iter()
        .find(|p| p.id == id)
        .context("插件未声明该页面")?;
    let permission = page
        .permission
        .as_deref()
        .map(|p| super::services::permission(source, p));
    ensure!(
        permitted(permission.as_deref(), &session.permissions),
        "当前用户无页面访问权限"
    );
    Ok((source, digest, generation, page))
}

pub(in crate::runtime::server) async fn mount(
    state: &RuntimeState,
    headers: &HeaderMap,
    session: &SessionContext,
    id: &str,
) -> Result<MountResponse> {
    let _permit = state.frontend.request_slot()?;
    let (source, revision, generation, page) = page(state, session, id).await?;
    let bundle = state
        .components()?
        .bundle(source, &session.tenant_id)
        .await?;
    ensure!(bundle.digest() == revision, "插件安装版本发生变化");
    let entry = page.entry;
    let token = state.frontend.issue(FrontendGrant {
        cookie: headers
            .get(header::COOKIE)
            .cloned()
            .context("会话 Cookie 缺失")?,
        session_id: session.session_id.clone(),
        tenant_id: session.tenant_id.clone(),
        user_id: session.user_id.clone(),
        page_id: id.into(),
        source_id: source.to_string(),
        activation_generation: generation.clone(),
        revision: revision.clone(),
        entry: entry.clone(),
        frontend_path: bundle.manifest().plugin.frontend.path.clone(),
        assets: Default::default(),
        issued: Instant::now(),
    })?;
    Ok(MountResponse {
        src: format!("/api/runtime/components/assets/{token}/{entry}"),
        token,
        revision,
        generation,
        session_context: session_context(session),
        context: tenant_context(session)?,
        assets: Default::default(),
        abi: Some(2),
    })
}

async fn validate(
    state: &RuntimeState,
    session: &SessionContext,
    token: &str,
) -> Result<FrontendGrant, RuntimeError> {
    let grant = state
        .frontend
        .get(token)
        .map_err(|_| RuntimeError::unauthorized("挂载已过期"))?;
    if grant.session_id != session.session_id
        || grant.tenant_id != session.tenant_id
        || grant.user_id != session.user_id
    {
        return Err(RuntimeError::forbidden("挂载不属于当前登录或租户"));
    }
    let (source, digest, generation, page) = page(state, session, &grant.page_id).await?;
    if source.to_string() != grant.source_id
        || digest != grant.revision
        || generation != grant.activation_generation
        || page.entry != grant.entry
    {
        return Err(RuntimeError::forbidden("插件版本已变化，请重新挂载"));
    }
    Ok(grant)
}

pub(super) async fn asset(
    State(state): State<RuntimeState>,
    Path((token, path)): Path<(String, String)>,
) -> Result<Response, RuntimeError> {
    az_plugin_bundle::validate_relative_path(&path)?;
    let _permit = state.frontend.request_slot()?;
    let grant = state
        .frontend
        .get(&token)
        .map_err(|_| RuntimeError::unauthorized("挂载已过期"))?;
    let mut cookie = HeaderMap::new();
    cookie.insert(header::COOKIE, grant.cookie.clone());
    let session = authenticate(&state, &cookie).await?;
    let grant = validate(&state, &session, &token).await?;
    let bundle = state
        .components()?
        .bundle(
            Uuid::parse_str(&grant.source_id).context("来源无效")?,
            &session.tenant_id,
        )
        .await?;
    if bundle.digest() != grant.revision {
        return Err(RuntimeError::forbidden("插件版本已撤销"));
    }
    let bytes = bundle
        .frontend(&path)
        .ok_or_else(|| RuntimeError::not_found("资产不属于当前插件包"))?;
    let prefix = format!(
        "{}/api/runtime/components/assets/{token}/",
        state.frontend.origin
    );
    let bytes = if path == grant.entry {
        render(bytes, &prefix, &path)?
    } else {
        bytes.to_vec()
    };
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(
            mime_guess::from_path(&path)
                .first_or_octet_stream()
                .as_ref(),
        )?,
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache, no-transform"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY,HeaderValue::from_str(&format!("sandbox allow-scripts allow-forms; default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' {prefix}; connect-src {prefix} blob:; style-src 'unsafe-inline' {prefix}; img-src data: blob: {prefix}; font-src data: {prefix}; object-src 'none'; frame-src 'none'; worker-src blob:; base-uri {prefix}; form-action 'none'; frame-ancestors 'self'"))?);
    Ok(response)
}

fn render(bytes: &[u8], prefix: &str, entry: &str) -> Result<Vec<u8>> {
    let document = kuchikiki::parse_html()
        .one(std::str::from_utf8(bytes)?)
        .document_node;
    for node in document
        .select("base")
        .map_err(|_| anyhow::anyhow!("解析 HTML 失败"))?
    {
        node.as_node().detach();
    }
    let base = kuchikiki::parse_html()
        .one("<head><base></head>")
        .document_node
        .select_first("base")
        .map_err(|_| anyhow::anyhow!("创建 base 失败"))?;
    let directory = entry
        .rsplit_once('/')
        .map(|(dir, _)| format!("{dir}/"))
        .unwrap_or_default();
    base.attributes
        .borrow_mut()
        .insert("href", format!("{prefix}{directory}"));
    let node = base.as_node().clone();
    node.detach();
    let head = document
        .select_first("head")
        .map_err(|_| anyhow::anyhow!("入口缺少 head"))?
        .as_node()
        .clone();
    let script = kuchikiki::parse_html()
        .one("<script></script>")
        .document_node
        .select_first("script")
        .map_err(|_| anyhow::anyhow!("创建 SDK 脚本失败"))?
        .as_node()
        .clone();
    script.detach();
    script.append(kuchikiki::NodeRef::new_text(
        az_plugin_runtime::FRONTEND_GUEST,
    ));
    head.prepend(script);
    head.prepend(node);
    let mut output = Vec::new();
    document.serialize(&mut output)?;
    Ok(output)
}

pub(super) async fn request(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(request): Json<model::Request>,
) -> Result<Json<RuntimeResponse<model::Response>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let _permit = state.frontend.request_slot()?;
    let grant = validate(&state, &session, &token).await?;
    let components = state.components()?;
    let authorization = components.services.enter(&session)?;
    let response = components
        .handle(
            Uuid::parse_str(&grant.source_id).context("来源无效")?,
            &session.tenant_id,
            &grant.revision,
            request.try_into()?,
            authorization.context.clone(),
        )
        .await?;
    Ok(Json(RuntimeResponse {
        data: response.into(),
    }))
}

pub(super) async fn renew(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<StatusCode, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    validate(&state, &session, &token).await?;
    state.frontend.renew(&token)?;
    Ok(StatusCode::NO_CONTENT)
}
