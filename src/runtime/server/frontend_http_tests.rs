use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, ensure};
use axum::http::{HeaderMap, HeaderValue, header};
use az_plugin_package::PluginPackage;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};

use super::{
    RuntimeState,
    frontend_access::FrontendAccess,
    process::ProcessManager,
    repository::{DiscoveredPlugin, RepositoryInstaller},
    store::PluginStore,
    wasm::WasmManager,
};

#[tokio::test]
#[ignore = "需要独立 AIO_TEST_DATABASE_URL、同值 AIO_DATABASE_URL 和测试启动账号"]
async fn frontend_http_mounts_verified_assets_and_revokes_live_access() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    ensure!(
        std::env::var("AIO_DATABASE_URL").as_deref() == Ok(database.as_str()),
        "测试必须使用同一独立数据库"
    );
    let temporary = tempfile::tempdir()?;
    let mut catalog = dill::CatalogBuilder::new();
    aio_plugin_identity_server::register(&mut catalog)?;
    let catalog = catalog.build();
    let identity = aio_plugin_identity_server::service(&catalog)?;
    identity.initialize().await?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&database)
        .await?;
    let store = Arc::new(PluginStore::new(pool.clone()));
    store.migrate().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let state = RuntimeState {
        components: None,
        store,
        repository: Arc::new(RepositoryInstaller::new(temporary.path().join("cache"))),
        identity,
        marketplace_url: String::new(),
        marketplace_syncing: Arc::new(Mutex::new(HashSet::new())),
        activation_locks: Arc::new(Mutex::new(HashMap::new())),
        publication_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        frontend: Arc::new(FrontendAccess::new(&base)?),
        process: Arc::new(ProcessManager::new()?),
        wasm: Arc::new(WasmManager::new()?),
    };
    let router = super::router(state.clone()).merge(aio_plugin_identity_server::router(&catalog)?);
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let result = exercise(&state, &base).await;
    server.abort();
    let _ = server.await;
    pool.close().await;
    result
}

async fn exercise(state: &RuntimeState, base: &str) -> Result<()> {
    let client = Client::new();
    let login = client.post(format!("{base}/api/auth/login")).json(&json!({
        "account": std::env::var("AIO_BOOTSTRAP_ACCOUNT")?, "password": std::env::var("AIO_BOOTSTRAP_PASSWORD")?
    })).send().await?.error_for_status()?;
    let cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .context("登录缺少 Cookie")?
        .to_str()?
        .split(';')
        .next()
        .context("登录 Cookie 为空")?
        .to_owned();
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, HeaderValue::from_str(&cookie)?);
    let session = state
        .identity
        .authenticate(&headers)
        .await?
        .context("测试会话无效")?;
    let page_id = format!("frontend-{}", uuid::Uuid::new_v4().simple());
    let git = format!("https://example.com/{page_id}.git");
    let manifest = format!(
        "[plugin.runtime]\nkind='page-definition'\nartifact='dist/pages.json'\n[plugin.frontend]\npath='dist/web'\n[plugin.marketplace]\ntitle='Frontend Integration'\nsummary='Isolated frontend test'\nlicense='MIT'\ntags=['test']\n[[plugin.subplugins]]\nid='screen'\npages=['{page_id}']\n"
    );
    let pages = json!([{"id": page_id, "label":"Frontend", "scene":{"id":"community","label":"Community"}, "body":{"kind":"frontend","entry":"index.html"}}]);
    let first = fixture(&git, "1.0.0", &manifest, &pages, b"<!doctype html><html><head></head><body>first<script src='assets/app.js'></script></body></html>")?;
    activate(state, &session.tenant_id, &page_id, &first).await?;
    let mounted = mount(&client, base, &cookie, &page_id).await?;
    assert_eq!(
        mounted["data"]["session_context"],
        super::request_context::session_context(&session)
    );
    assert_eq!(
        mounted["data"]["context"],
        super::request_context::tenant_context(&session)?
    );
    assert!(
        mounted["data"]["assets"]["assets/app.js"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64)
    );
    let src = mounted["data"]["src"].as_str().context("缺少 src")?;
    let token = mounted["data"]["token"].as_str().context("缺少 token")?;
    let response = client
        .get(format!("{base}{src}"))
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store, no-transform"
    );
    assert!(
        response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()?
            .contains("sandbox allow-scripts;")
    );
    assert!(!response.headers().contains_key(header::SET_COOKIE));
    let html = response.text().await?;
    assert!(html.contains("__aio_bridge.js"));
    assert!(html.contains("first"));
    let root = format!("{base}/api/runtime/frontend/assets/{token}");
    let lease = format!("{base}/api/runtime/frontend/{token}/renew");
    assert_eq!(
        client.post(&lease).send().await?.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .post(&lease)
            .header(header::COOKIE, &cookie)
            .send()
            .await?
            .status(),
        StatusCode::NO_CONTENT
    );
    let cached = super::frontend_package::prepare(state, &first.rev, "index.html").await?;
    let reused = super::frontend_package::prepare(state, &first.rev, "index.html").await?;
    assert!(Arc::ptr_eq(&cached, &reused), "已验证版本不能再次解包");
    let asset = client.get(format!("{root}/assets/app.js")).send().await?;
    let etag = asset.headers()[header::ETAG].clone();
    assert_eq!(
        client
            .get(format!("{root}/assets/app.js"))
            .header(header::IF_NONE_MATCH, &etag)
            .send()
            .await?
            .status(),
        StatusCode::NOT_MODIFIED
    );
    assert_eq!(
        client
            .get(format!("{root}/assets/app.js"))
            .send()
            .await?
            .status(),
        StatusCode::OK
    );
    let script = state
        .repository
        .artifact(&first.rev, "dist/web/assets/app.js")?;
    tokio::fs::write(&script, b"modified").await?;
    assert_eq!(
        client
            .get(format!("{root}/assets/app.js"))
            .send()
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    tokio::fs::write(&script, b"window.testReady=true;").await?;
    assert_eq!(
        client
            .get(format!("{root}/dist/pages.json"))
            .send()
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .post(format!("{base}/api/runtime/frontend/{token}/request"))
            .json(&json!({"method":"GET","path":"/counter","body":""}))
            .send()
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut expired_permission = pages.clone();
    expired_permission[0]["required_permission"] = json!("permission:not-granted");
    sqlx::query("UPDATE plugin_revisions SET pages=$1 WHERE source_id=$2")
        .bind(expired_permission)
        .bind(&page_id)
        .execute(&state.store.pool)
        .await?;
    assert_eq!(
        client.get(format!("{base}{src}")).send().await?.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .post(&lease)
            .header(header::COOKIE, &cookie)
            .send()
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .get(format!("{root}/assets/app.js"))
            .header(header::IF_NONE_MATCH, &etag)
            .send()
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE plugin_revisions SET pages=$1 WHERE source_id=$2")
        .bind(&pages)
        .bind(&page_id)
        .execute(&state.store.pool)
        .await?;
    state
        .store
        .set_enabled(&session.tenant_id, &page_id, false, None)
        .await?;
    assert_eq!(
        client.get(format!("{base}{src}")).send().await?.status(),
        StatusCode::NOT_FOUND
    );
    state
        .store
        .set_enabled(&session.tenant_id, &page_id, true, None)
        .await?;
    assert_eq!(
        client.get(format!("{base}{src}")).send().await?.status(),
        StatusCode::FORBIDDEN
    );
    let second = fixture(
        &git,
        "1.0.1",
        &manifest,
        &pages,
        b"<!doctype html><html><head></head><body>second</body></html>",
    )?;
    activate(state, &session.tenant_id, &page_id, &second).await?;
    assert_eq!(
        client.get(format!("{base}{src}")).send().await?.status(),
        StatusCode::FORBIDDEN
    );
    let current = mount(&client, base, &cookie, &page_id).await?;
    let current_token = current["data"]["token"]
        .as_str()
        .context("缺少当前 token")?;
    let current_src = current["data"]["src"].as_str().context("缺少当前 src")?;
    assert_eq!(
        client
            .delete(format!("{base}/api/runtime/frontend/{current_token}"))
            .header(header::COOKIE, &cookie)
            .send()
            .await?
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .get(format!("{base}{current_src}"))
            .send()
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let switched = mount(&client, base, &cookie, &page_id).await?;
    let other_tenant = format!("frontend-tenant-{}", uuid::Uuid::new_v4().simple());
    sqlx::query("INSERT INTO tenants (id,label) VALUES ($1,'Frontend Other')")
        .bind(&other_tenant)
        .execute(&state.store.pool)
        .await?;
    sqlx::query("UPDATE auth_sessions SET tenant_id=$1 WHERE id=$2")
        .bind(&other_tenant)
        .bind(&session.session_id)
        .execute(&state.store.pool)
        .await?;
    assert_eq!(
        client
            .get(format!(
                "{base}{}",
                switched["data"]["src"].as_str().context("缺少切换前 src")?
            ))
            .send()
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE auth_sessions SET tenant_id=$1 WHERE id=$2")
        .bind(&session.tenant_id)
        .bind(&session.session_id)
        .execute(&state.store.pool)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id=$1")
        .bind(other_tenant)
        .execute(&state.store.pool)
        .await?;
    state.store.uninstall(&session.tenant_id, &page_id).await?;
    assert_eq!(
        client
            .get(format!(
                "{base}{}",
                switched["data"]["src"].as_str().context("缺少卸载前 src")?
            ))
            .send()
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    activate(state, &session.tenant_id, &page_id, &second).await?;
    let logged_out = mount(&client, base, &cookie, &page_id).await?;
    state.identity.logout(&headers).await?;
    assert_eq!(
        client
            .get(format!(
                "{base}{}",
                logged_out["data"]["src"]
                    .as_str()
                    .context("缺少退出前 src")?
            ))
            .send()
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    state.store.uninstall(&session.tenant_id, &page_id).await?;
    Ok(())
}

async fn mount(client: &Client, base: &str, cookie: &str, page: &str) -> Result<Value> {
    let response = client
        .post(format!("{base}/api/runtime/frontend/mount"))
        .header(header::COOKIE, cookie)
        .json(&json!({"page_id":page}))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    ensure!(status.is_success(), "挂载失败 {status}: {text}");
    Ok(serde_json::from_str(&text)?)
}

fn fixture(
    git: &str,
    version: &str,
    manifest: &str,
    pages: &Value,
    html: &[u8],
) -> Result<PluginPackage> {
    PluginPackage::new(
        git.to_owned(),
        version.to_owned(),
        None,
        manifest.to_owned(),
        &serde_json::to_vec(pages)?,
        [
            ("index.html".to_owned(), html.to_vec()),
            (
                "assets/app.js".to_owned(),
                b"window.testReady=true;".to_vec(),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

async fn activate(
    state: &RuntimeState,
    tenant: &str,
    source: &str,
    package: &PluginPackage,
) -> Result<()> {
    state.store.save_package(package).await?;
    state.repository.stage_publish(package).await?;
    let verified = package.verify()?;
    state
        .store
        .activate(
            tenant,
            DiscoveredPlugin {
                source_id: source.to_owned(),
                git: package.git.clone(),
                revision: package.rev.clone(),
                runtime: crate::runtime::PluginRuntime::PageDefinition,
                manifest: serde_json::to_value(verified.manifest.plugin)?,
                pages: serde_json::from_slice(&verified.artifact)?,
                artifact: "dist/pages.json".to_owned(),
            },
            None,
            None,
        )
        .await
}
