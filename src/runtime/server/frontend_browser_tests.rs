use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, ensure};

use super::{
    RuntimeState, frontend_access::FrontendAccess, process::ProcessManager,
    repository::RepositoryInstaller, store::PluginStore, wasm::WasmManager,
};

#[tokio::test]
#[ignore = "需要测试 PostgreSQL、已构建 Web 和 Dioxus 联合插件，以及 Node/Playwright"]
async fn mounts_real_dioxus_binary_and_calls_component_over_browser_bridge() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    ensure!(
        std::env::var("AIO_DATABASE_URL").as_deref() == Ok(database.as_str()),
        "测试数据库不一致"
    );
    let temporary = tempfile::tempdir()?;
    let mut catalog = dill::CatalogBuilder::new();
    aio_plugin_identity_server::register(&mut catalog)?;
    aio_plugin_tenant_server::register(&mut catalog)?;
    let catalog = catalog.build();
    let identity = aio_plugin_identity_server::service(&catalog)?;
    identity.initialize().await?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&database)
        .await?;
    let store = Arc::new(PluginStore::new(pool.clone()));
    store.migrate().await?;
    let tenant_id = format!("frontend-test-{}", uuid::Uuid::new_v4().simple());
    let account = std::env::var("AIO_BOOTSTRAP_ACCOUNT")?;
    let (user_id, display_name) = sqlx::query_as::<_, (String, String)>(
        "SELECT id, display_name FROM identity_users WHERE account = $1",
    )
    .bind(&account)
    .fetch_one(&pool)
    .await?;
    let mut transaction = pool.begin().await?;
    sqlx::query("INSERT INTO tenants (id, label) VALUES ($1, 'Dioxus Browser Test')")
        .bind(&tenant_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO tenant_memberships (tenant_id, user_id, display_name) VALUES ($1, $2, $3)",
    )
    .bind(&tenant_id)
    .bind(&user_id)
    .bind(display_name)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO tenant_member_roles (tenant_id, user_id, role_id) VALUES ($1, $2, 'tenant-admin')")
        .bind(&tenant_id)
        .bind(&user_id)
        .execute(&mut *transaction)
        .await?;
    for permission in ["plugin:manage", "tenant:manage", "rbac:manage"] {
        sqlx::query("INSERT INTO role_permissions (tenant_id, role_id, permission) VALUES ($1, 'tenant-admin', $2)")
            .bind(&tenant_id)
            .bind(permission)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let state = RuntimeState {
        components: None,
        store,
        repository: Arc::new(RepositoryInstaller::new(temporary.path().join("cache"))),
        identity,
        marketplace_url: String::new(),
        marketplace_syncing: Arc::new(Mutex::new(HashSet::from([String::new()]))),
        activation_locks: Arc::new(Mutex::new(HashMap::new())),
        publication_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        frontend: Arc::new(FrontendAccess::new(&base)?),
        process: Arc::new(ProcessManager::new()?),
        wasm: Arc::new(WasmManager::new()?),
    };
    let dist = std::path::PathBuf::from("target/dx/aio-idea/release/web/public");
    ensure!(dist.join("index.html").is_file(), "先构建 Web 宿主");
    let router = super::router(state)
        .merge(aio_plugin_identity_server::router(&catalog)?)
        .merge(aio_plugin_tenant_server::router(&catalog)?)
        .fallback_service(tower_http::services::ServeDir::new(&dist).fallback(
            tower_http::services::ServeFile::new(dist.join("index.html")),
        ))
        .layer(axum::middleware::map_response(
            |mut response: axum::response::Response| async move {
                response.headers_mut().append(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    axum::http::HeaderValue::from_static("frame-src 'self'; object-src 'none'"),
                );
                response
            },
        ));
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let result = tokio::process::Command::new(
        std::env::var("AIO_TEST_NODE").unwrap_or_else(|_| "node".to_owned()),
    )
    .arg("tests/browser/frontend.cjs")
    .env("AIO_URL", &base)
    .env("AIO_TEST_OUTPUT", temporary.path())
    .env("AIO_TEST_TENANT", &tenant_id)
    .output()
    .await
    .context("启动前端浏览器测试失败");
    server.abort();
    let _ = server.await;
    for statement in [
        "DELETE FROM plugin_runtime_instances WHERE tenant_id = $1",
        "DELETE FROM plugin_page_states WHERE tenant_id = $1",
        "DELETE FROM tenant_plugin_bindings WHERE tenant_id = $1",
        "DELETE FROM plugin_lifecycle_events WHERE tenant_id = $1",
        "DELETE FROM plugin_registries WHERE tenant_id = $1",
        "DELETE FROM plugin_publish_credentials WHERE tenant_id = $1",
        "DELETE FROM plugin_publish_jobs WHERE tenant_id = $1",
        "DELETE FROM auth_sessions WHERE tenant_id = $1",
        "DELETE FROM tenant_member_roles WHERE tenant_id = $1",
        "DELETE FROM role_permissions WHERE tenant_id = $1",
        "DELETE FROM tenant_memberships WHERE tenant_id = $1",
        "DELETE FROM tenants WHERE id = $1",
    ] {
        sqlx::query(statement)
            .bind(&tenant_id)
            .execute(&pool)
            .await?;
    }
    pool.close().await;
    let output = result?;
    ensure!(
        output.status.success(),
        "浏览器测试失败: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}
