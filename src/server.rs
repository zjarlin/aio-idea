use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context as _, Result};
use axum::{Router, routing::get};
use tower_http::services::{ServeDir, ServeFile};

use crate::plugins;

pub async fn run() -> Result<()> {
    let port = env::var("AIO_WEB_PORT")
        .unwrap_or_else(|_| "8080".to_owned())
        .parse::<u16>()
        .context("AIO_WEB_PORT 必须是有效端口")?;
    let web_dist = env::var_os("AIO_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/dx/aio-public-shell/release/web/public"));
    let index = web_dist.join("index.html");
    let application = ServeDir::new(web_dist).fallback(ServeFile::new(index));
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(plugins::server_router()?)
        .fallback_service(application);
    let address = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("绑定监听地址失败: {address}"))?;
    println!("AIO listening on http://{}", listener.local_addr()?);
    axum::serve(listener, router).await.context("AIO 服务异常退出")
}
