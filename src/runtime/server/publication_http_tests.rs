use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, ensure};
use az_plugin_package::{PACKAGE_CONTENT_TYPE, PluginPackage};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::process::Command;

use super::{
    RuntimeState, process::ProcessManager, repository::RepositoryInstaller, store::PluginStore,
    wasm::WasmManager,
};

const PAGES: &str = r#"[{"id":"binary-integration","label":"Binary Integration","scene":{"id":"community","label":"Community"},"body":{"kind":"text","title":"Binary","content":"first"}}]"#;
const MANIFEST: &str = "[plugin.runtime]\nkind='page-definition'\nartifact='dist/pages.json'\n[plugin.marketplace]\ntitle='Binary Integration'\nsummary='Binary publication test'\nlicense='MIT'\ntags=['test']\n[[plugin.subplugins]]\nid='integration'\npages=['binary-integration']\n";

#[tokio::test]
#[ignore = "需要独立 AIO_TEST_DATABASE_URL、同值 AIO_DATABASE_URL、AIO_TEST_CLI 和测试启动账号"]
async fn binary_cli_publishes_downloads_recovers_and_rolls_back_over_http() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    ensure!(
        std::env::var("AIO_DATABASE_URL").as_deref() == Ok(database.as_str()),
        "测试与身份数据库必须是同一独立数据库"
    );
    let cli = std::env::var("AIO_TEST_CLI")?;
    let temp = tempfile::tempdir()?;
    let cache = temp.path().join("cache");
    let mut catalog = dill::CatalogBuilder::new();
    aio_plugin_identity_server::register(&mut catalog)?;
    aio_plugin_tenant_server::register(&mut catalog)?;
    let catalog = catalog.build();
    let identity = aio_plugin_identity_server::service(&catalog)?;
    identity.initialize().await?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&database)
        .await?;
    let store = Arc::new(PluginStore::new(pool.clone()));
    store.migrate().await?;
    let state = RuntimeState {
        store,
        repository: Arc::new(RepositoryInstaller::new(cache.clone())),
        identity,
        marketplace_url: String::new(),
        marketplace_syncing: Arc::new(Mutex::new(HashSet::new())),
        activation_locks: Arc::new(Mutex::new(HashMap::new())),
        publication_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        process: Arc::new(ProcessManager::new()?),
        wasm: Arc::new(WasmManager::new()?),
    };
    let router = super::router(state.clone())
        .merge(aio_plugin_identity_server::router(&catalog)?)
        .merge(aio_plugin_tenant_server::router(&catalog)?);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let result = exercise_http(&state, &base, &cli, temp.path(), &cache, &pool).await;
    server.abort();
    let _ = server.await;
    pool.close().await;
    result
}

async fn exercise_http(
    state: &RuntimeState,
    base: &str,
    cli: &str,
    root: &Path,
    cache: &Path,
    pool: &sqlx::PgPool,
) -> Result<()> {
    let client = Client::new();
    let password = std::env::var("AIO_BOOTSTRAP_PASSWORD")?;
    let account = std::env::var("AIO_BOOTSTRAP_ACCOUNT").unwrap_or_else(|_| "admin".to_owned());
    let login = client
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"account": account, "password": password}))
        .send()
        .await?;
    ensure!(
        login.status().is_success(),
        "测试账号登录失败: {}",
        login.status()
    );
    let cookie = login
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .context("测试登录缺少 Cookie")?
        .to_str()?
        .split(';')
        .next()
        .context("登录 Cookie 为空")?
        .to_owned();
    let git = format!(
        "https://example.com/binary-{}.git",
        uuid::Uuid::new_v4().simple()
    );
    let page_id = format!("binary-{}", uuid::Uuid::new_v4().simple());
    let pages = PAGES.replace("binary-integration", &page_id);
    let manifest = MANIFEST.replace("binary-integration", &page_id);
    let credential = request_json(
        &client,
        "POST",
        &format!("{base}/api/runtime/publish-credentials"),
        &cookie,
        Some(json!({"git": git})),
    )
    .await?;
    let token = credential["data"]["token"]
        .as_str()
        .context("缺少发布凭证")?;
    let endpoint = format!("{base}/api/runtime/plugins/publish");
    assert_eq!(upload_status_without_body(base, None).await?, 401);
    let permits = state.publication_slots.acquire_many(2).await?;
    assert_eq!(upload_status_without_body(base, Some(&cookie)).await?, 503);
    drop(permits);
    let plugin = root.join("plugin");
    tokio::fs::create_dir_all(plugin.join("dist")).await?;
    tokio::fs::write(plugin.join("aio-plugin.toml"), &manifest).await?;
    tokio::fs::write(plugin.join("dist/pages.json"), &pages).await?;
    let first_file = root.join("first.aio-plugin");
    package_cli(cli, &plugin, &git, "1.0.0", &first_file).await?;
    let first_bytes = tokio::fs::read(&first_file).await?;
    let first = PluginPackage::decode(&first_bytes)?;
    assert!(first.source_revision.is_none());
    assert!(!plugin.join(".git").exists());
    let anonymous = client
        .post(&endpoint)
        .header(reqwest::header::CONTENT_TYPE, PACKAGE_CONTENT_TYPE)
        .body(first_bytes.clone())
        .send()
        .await?;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    publish_cli(cli, &first_file, &endpoint, token, root).await?;
    let source = active_revision(&client, base, &cookie, &first.rev).await?;
    let download = client
        .get(format!("{base}/api/runtime/packages/{}", first.rev))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    assert_eq!(PluginPackage::decode(&download)?, first);
    let spoof = PluginPackage::new(
        "https://example.com/other.git".to_owned(),
        "1.0.0".to_owned(),
        None,
        manifest.clone(),
        pages.as_bytes(),
        Default::default(),
    )?;
    let denied = client
        .post(&endpoint)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, PACKAGE_CONTENT_TYPE)
        .body(spoof.encode()?)
        .send()
        .await?;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    tokio::fs::write(
        plugin.join("dist/pages.json"),
        pages.replace("first", "second"),
    )
    .await?;
    let conflict_file = root.join("conflict.aio-plugin");
    package_cli(cli, &plugin, &git, "1.0.0", &conflict_file).await?;
    assert!(
        publish_cli(cli, &conflict_file, &endpoint, token, root)
            .await
            .is_err()
    );
    active_revision(&client, base, &cookie, &first.rev).await?;
    let second_file = root.join("second.aio-plugin");
    package_cli(cli, &plugin, &git, "1.0.1", &second_file).await?;
    let second = PluginPackage::decode(&tokio::fs::read(&second_file).await?)?;
    publish_cli(cli, &second_file, &endpoint, token, root).await?;
    active_revision(&client, base, &cookie, &second.rev).await?;
    request_json(
        &client,
        "POST",
        &format!("{base}/api/runtime/plugins/{source}/rollback"),
        &cookie,
        None,
    )
    .await?;
    active_revision(&client, base, &cookie, &first.rev).await?;
    request_json(
        &client,
        "POST",
        &format!("{base}/api/runtime/plugins/{source}/uninstall"),
        &cookie,
        None,
    )
    .await?;
    tokio::fs::remove_dir_all(cache).await?;
    request_json(
        &client,
        "POST",
        &format!("{base}/api/runtime/plugins/install"),
        &cookie,
        Some(json!({"git": git, "rev": first.rev})),
    )
    .await?;
    active_revision(&client, base, &cookie, &first.rev).await?;
    assert!(
        state
            .repository
            .artifact(&first.rev, "dist/pages.json")?
            .is_file()
    );
    assert_eq!(
        state
            .store
            .published_marketplace_entry(&git, None)
            .await?
            .context("缺少市场当前发布版本")?
            .rev,
        second.rev
    );
    let invalid = PluginPackage::new(
        git.clone(),
        "1.0.2".to_owned(),
        None,
        manifest.clone(),
        b"[]",
        Default::default(),
    )?;
    let rejected = client
        .post(&endpoint)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, PACKAGE_CONTENT_TYPE)
        .body(invalid.encode()?)
        .send()
        .await?;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    active_revision(&client, base, &cookie, &first.rev).await?;
    let frontend = PluginPackage::new(
        git.clone(),
        "1.0.3".to_owned(),
        None,
        format!("{manifest}\n[plugin.frontend]\npath='dist/web'\n"),
        pages.as_bytes(),
        [("index.html".to_owned(), b"<!doctype html>".to_vec())]
            .into_iter()
            .collect(),
    )?;
    let rejected = client
        .post(&endpoint)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, PACKAGE_CONTENT_TYPE)
        .body(frontend.encode()?)
        .send()
        .await?;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert!(rejected.text().await?.contains("隔离挂载"));
    active_revision(&client, base, &cookie, &first.rev).await?;
    assert!(state.store.package_archive(&frontend.rev).await?.is_none());
    let tenant = format!("tenant_{}", uuid::Uuid::new_v4().simple());
    sqlx::query("INSERT INTO tenants (id, label) VALUES ($1, 'Other Tenant')")
        .bind(&tenant)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO tenant_memberships (tenant_id, user_id, display_name) SELECT $1, id, display_name FROM identity_users WHERE account = $2")
        .bind(&tenant).bind(&account).execute(pool).await?;
    request_json(
        &client,
        "POST",
        &format!("{base}/api/tenants/switch"),
        &cookie,
        Some(json!({"tenant_id": tenant})),
    )
    .await?;
    let other = request_json(
        &client,
        "GET",
        &format!("{base}/api/runtime/catalog"),
        &cookie,
        None,
    )
    .await?;
    assert_eq!(other["data"]["tenant"]["id"], tenant);
    assert!(
        other["data"]["plugins"]
            .as_array()
            .context("缺少插件列表")?
            .is_empty()
    );
    Ok(())
}

async fn upload_status_without_body(base: &str, cookie: Option<&str>) -> Result<u16> {
    let address = base.trim_start_matches("http://").to_owned();
    let cookie = cookie
        .map(|cookie| format!("Cookie: {cookie}\r\n"))
        .unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        use std::io::{Read as _, Write as _};
        let mut connection = std::net::TcpStream::connect(&address)?;
        connection.set_read_timeout(Some(std::time::Duration::from_secs(3)))?;
        connection.write_all(format!("POST /api/runtime/plugins/publish HTTP/1.1\r\nHost: {address}\r\nContent-Type: {PACKAGE_CONTENT_TYPE}\r\nContent-Length: 50331648\r\n{cookie}\r\n").as_bytes())?;
        let mut response = [0_u8; 1024];
        let count = connection.read(&mut response)?;
        let response = std::str::from_utf8(&response[..count])?;
        response.split_whitespace().nth(1).context("上传入口未提前返回 HTTP 状态")?.parse().context("上传入口状态无效")
    }).await.context("等待无正文上传检测失败")?
}

async fn package_cli(
    cli: &str,
    plugin: &Path,
    git: &str,
    version: &str,
    output: &Path,
) -> Result<()> {
    let result = Command::new(cli)
        .args(["plugin", "package"])
        .arg(plugin)
        .args(["--git", git, "--version", version, "--output"])
        .arg(output)
        .output()
        .await?;
    ensure!(
        result.status.success(),
        "CLI 打包失败: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

async fn publish_cli(
    cli: &str,
    package: &Path,
    endpoint: &str,
    token: &str,
    cwd: &Path,
) -> Result<()> {
    let result = Command::new(cli)
        .args(["plugin", "publish"])
        .arg(package)
        .current_dir(cwd)
        .env("AIO_PLUGIN_PUBLISH_URL", endpoint)
        .env("AIO_PLUGIN_PUBLISH_TOKEN", token)
        .output()
        .await?;
    ensure!(
        result.status.success(),
        "CLI 上传失败: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

async fn request_json(
    client: &Client,
    method: &str,
    url: &str,
    cookie: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut request = client
        .request(method.parse()?, url)
        .header(reqwest::header::COOKIE, cookie);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    if status == StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    let result: Value = response.json().await?;
    ensure!(status.is_success(), "请求失败: {status} {result}");
    Ok(result)
}

async fn active_revision(
    client: &Client,
    base: &str,
    cookie: &str,
    revision: &str,
) -> Result<String> {
    let catalog = request_json(
        client,
        "GET",
        &format!("{base}/api/runtime/catalog"),
        cookie,
        None,
    )
    .await?;
    let plugin = catalog["data"]["plugins"]
        .as_array()
        .context("缺少插件目录")?
        .iter()
        .find(|plugin| plugin["revision"] == revision)
        .context("期望的活动版本不在目录中")?;
    Ok(plugin["source_id"]
        .as_str()
        .context("插件缺少来源")?
        .to_owned())
}
