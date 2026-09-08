use std::{
    env,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use anyhow::{Context as _, Result};
use axum::{
    Json, Router,
    http::StatusCode,
    routing::{any, get},
};
use tower_http::services::{ServeDir, ServeFile};

use crate::{plugins, runtime};

pub async fn run() -> Result<()> {
    let port = env::var("AIO_WEB_PORT")
        .unwrap_or_else(|_| "8080".to_owned())
        .parse::<u16>()
        .context("AIO_WEB_PORT 必须是有效端口")?;
    let host = env::var("AIO_WEB_HOST")
        .unwrap_or_else(|_| "127.0.0.1".to_owned())
        .parse::<IpAddr>()
        .context("AIO_WEB_HOST 必须是有效 IP 地址")?;
    let web_dist = env::var_os("AIO_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/dx/aio-public-shell/release/web/public"));
    let index = web_dist.join("index.html");
    let application = ServeDir::new(web_dist).fallback(ServeFile::new(index));
    let plugin_catalog = plugins::server_catalog()?;
    let identity = aio_plugin_identity_server::service(&plugin_catalog)?;
    identity.initialize().await?;
    aio_plugin_rbac_server::service(&plugin_catalog)?
        .initialize()
        .await?;
    let runtime = runtime::server::RuntimeState::initialize(identity).await?;
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(runtime::server::router(runtime))
        .merge(plugins::server_router(&plugin_catalog)?)
        .route("/api/{*path}", any(api_not_found))
        .fallback_service(application);
    let address = SocketAddr::from((host, port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("绑定监听地址失败: {address}"))?;
    println!("AIO listening on http://{}", listener.local_addr()?);
    axum::serve(listener, router)
        .await
        .context("AIO 服务异常退出")
}

async fn api_not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "API 路由不存在" })),
    )
}
