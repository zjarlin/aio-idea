mod manifest;
mod repository;
mod routes;
mod store;
mod wasm;

use std::{env, path::PathBuf, sync::Arc};

use anyhow::{Context as _, Result};
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;

pub use routes::router;

#[derive(Clone)]
pub struct RuntimeState {
    pub store: Arc<store::PluginStore>,
    pub repository: Arc<repository::RepositoryInstaller>,
    pub identity: Arc<aio_plugin_identity_server::IdentityService>,
    pub marketplace_url: String,
}

impl RuntimeState {
    pub async fn initialize(
        identity: Arc<aio_plugin_identity_server::IdentityService>,
    ) -> Result<Self> {
        let database_url = env::var("AIO_DATABASE_URL")
            .or_else(|_| env::var("AZ_AIO_DATABASE_URL"))
            .context("缺少 AIO_DATABASE_URL")?;
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&database_url)
            .await
            .context("连接插件运行时 PostgreSQL 失败")?;
        let store = Arc::new(store::PluginStore::new(pool));
        store.migrate().await?;
        let cache_root = env::var_os("AIO_PLUGIN_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".aio/runtime"));
        let repository = Arc::new(repository::RepositoryInstaller::new(cache_root));
        let state = Self {
            store,
            repository,
            identity,
            marketplace_url: env::var("AIO_MARKETPLACE_URL").unwrap_or_else(|_| {
                "https://raw.githubusercontent.com/zjarlin/aio/main/marketplace/index.json"
                    .to_owned()
            }),
        };
        state.ensure_default_plugins().await?;
        Ok(state)
    }

    async fn ensure_default_plugins(&self) -> Result<()> {
        if self.store.has_plugins("default").await? {
            return Ok(());
        }
        let path = env::var_os("AIO_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("aio.toml"));
        let content = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("读取壳配置失败: {}", path.display()))?;
        let configuration: ShellConfiguration =
            toml::from_str(&content).context("解析壳插件组合失败")?;
        let mut plugins = Vec::with_capacity(configuration.plugins.len());
        for source in configuration.plugins {
            plugins.push(
                self.repository
                    .discover(&source.git, Some(&source.rev))
                    .await?,
            );
        }
        for plugin in plugins {
            self.store.activate("default", plugin).await?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct ShellConfiguration {
    #[serde(default)]
    plugins: Vec<ConfiguredPlugin>,
}

#[derive(Deserialize)]
struct ConfiguredPlugin {
    git: String,
    rev: String,
}

#[cfg(test)]
mod tests {
    use super::ShellConfiguration;

    #[test]
    fn reads_default_git_combination() {
        let configuration: ShellConfiguration = toml::from_str(
            r#"
                [application]
                name = "aio"

                [[plugins]]
                git = "https://github.com/example/plugin.git"
                rev = "0123456789012345678901234567890123456789"
            "#,
        )
        .expect("配置应可解析");

        assert_eq!(configuration.plugins.len(), 1);
        assert_eq!(
            configuration.plugins[0].git,
            "https://github.com/example/plugin.git"
        );
    }
}
