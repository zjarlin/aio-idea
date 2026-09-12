use anyhow::{Context as _, Result, ensure};
use axum::http::header;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "需要独立 AIO_TEST_DATABASE_URL 与测试启动账号"]
async fn bootstrap_revalidates_permissions_and_logout_before_304() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    ensure!(
        database.ends_with("/aio_startup_test"),
        "启动测试只允许独立数据库"
    );
    ensure!(std::env::var("AIO_DATABASE_URL").as_deref() == Ok(database.as_str()));
    let temporary = tempfile::tempdir()?;
    let mut catalog = dill::CatalogBuilder::new();
    aio_plugin_identity_server::register(&mut catalog)?;
    let catalog = catalog.build();
    let identity = aio_plugin_identity_server::service(&catalog)?;
    identity.initialize().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let state =
        super::RuntimeState::isolated_admin_test(identity, &database, &base, temporary.path())
            .await?;
    let router = super::router(state.clone()).merge(aio_plugin_identity_server::router(&catalog)?);
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let result = exercise(&state, &base).await;
    server.abort();
    let _ = server.await;
    result
}

async fn exercise(state: &super::RuntimeState, base: &str) -> Result<()> {
    sqlx::query("INSERT INTO role_permissions (tenant_id, role_id, permission) VALUES ('default', 'platform-admin', 'plugin:manage') ON CONFLICT DO NOTHING")
        .execute(&state.store.pool).await?;
    let client = Client::new();
    let url = format!("{base}/api/runtime/bootstrap");
    let anonymous = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    assert!(anonymous["data"].is_null());
    let login = client
        .post(format!("{base}/api/auth/login"))
        .json(&json!({
            "account": std::env::var("AIO_BOOTSTRAP_ACCOUNT")?,
            "password": std::env::var("AIO_BOOTSTRAP_PASSWORD")?,
        }))
        .send()
        .await?
        .error_for_status()?;
    let cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .context("登录没有返回 Cookie")?
        .to_str()?
        .split(';')
        .next()
        .context("Cookie 为空")?
        .to_owned();
    let response = client
        .get(&url)
        .header(header::COOKIE, &cookie)
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert!(
        response.headers()["server-timing"]
            .to_str()?
            .contains("auth;dur=")
    );
    let first_etag = response.headers()[header::ETAG].clone();
    let first: Value = response.json().await?;
    assert!(
        first["data"]["permissions"]
            .as_array()
            .context("权限字段缺失")?
            .contains(&json!("plugin:manage"))
    );
    assert_eq!(
        client
            .get(&url)
            .header(header::COOKIE, &cookie)
            .header(header::IF_NONE_MATCH, &first_etag)
            .send()
            .await?
            .status(),
        StatusCode::NOT_MODIFIED
    );

    sqlx::query(
        "DELETE FROM role_permissions WHERE tenant_id='default' AND permission='plugin:manage'",
    )
    .execute(&state.store.pool)
    .await?;
    let changed = client
        .get(&url)
        .header(header::COOKIE, &cookie)
        .header(header::IF_NONE_MATCH, &first_etag)
        .send()
        .await?;
    assert_eq!(changed.status(), StatusCode::OK);
    assert_ne!(changed.headers()[header::ETAG], first_etag);
    let changed: Value = changed.json().await?;
    assert!(
        !changed["data"]["permissions"]
            .as_array()
            .context("权限字段缺失")?
            .contains(&json!("plugin:manage"))
    );
    assert_ne!(
        changed["data"]["catalog"]["context"],
        first["data"]["catalog"]["context"]
    );

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::COOKIE, cookie.parse()?);
    let session = state
        .identity
        .authenticate(&headers)
        .await?
        .context("测试会话丢失")?;
    let tenant = format!("startup-{}", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO tenants (id, label) VALUES ($1, 'Startup B')")
        .bind(&tenant)
        .execute(&state.store.pool)
        .await?;
    sqlx::query("INSERT INTO tenant_memberships (tenant_id, user_id, display_name) VALUES ($1, $2, 'Startup B')")
        .bind(&tenant).bind(&session.user_id).execute(&state.store.pool).await?;
    assert!(state.identity.switch_tenant(&session, &tenant).await?);
    let switched = client
        .get(&url)
        .header(header::COOKIE, &cookie)
        .header(header::IF_NONE_MATCH, &first_etag)
        .send()
        .await?;
    assert_eq!(switched.status(), StatusCode::OK);
    let switched: Value = switched.json().await?;
    assert_eq!(switched["data"]["catalog"]["tenant"]["id"], tenant);
    assert_eq!(switched["data"]["permissions"], json!([]));
    assert_ne!(
        switched["data"]["catalog"]["context"],
        first["data"]["catalog"]["context"]
    );

    client
        .post(format!("{base}/api/auth/logout"))
        .header(header::COOKIE, &cookie)
        .send()
        .await?
        .error_for_status()?;
    let revoked = client
        .get(&url)
        .header(header::COOKIE, &cookie)
        .header(header::IF_NONE_MATCH, &first_etag)
        .send()
        .await?;
    assert_eq!(revoked.status(), StatusCode::OK);
    assert!(revoked.json::<Value>().await?["data"].is_null());
    sqlx::query("DELETE FROM tenant_memberships WHERE tenant_id=$1")
        .bind(&tenant)
        .execute(&state.store.pool)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id=$1")
        .bind(&tenant)
        .execute(&state.store.pool)
        .await?;
    sqlx::query("INSERT INTO role_permissions (tenant_id, role_id, permission) VALUES ('default', 'platform-admin', 'plugin:manage') ON CONFLICT DO NOTHING")
        .execute(&state.store.pool).await?;
    Ok(())
}
