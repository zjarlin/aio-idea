mod lifecycle;
mod page_state;
mod process;
mod repository;
mod routes;
mod store;
mod supervisor;
mod wasm;

use std::{env, path::PathBuf, sync::Arc};

use anyhow::{Context as _, Result};
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;

pub use routes::router;
pub use supervisor::run as run_supervisor;

#[derive(Clone)]
pub struct RuntimeState {
    pub store: Arc<store::PluginStore>,
    pub repository: Arc<repository::RepositoryInstaller>,
    pub identity: Arc<aio_plugin_identity_server::IdentityService>,
    pub marketplace_url: String,
    pub process: Arc<process::ProcessManager>,
    pub wasm: Arc<wasm::WasmManager>,
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
        let process = Arc::new(process::ProcessManager::new()?);
        let wasm = Arc::new(wasm::WasmManager::new()?);
        let state = Self {
            store,
            repository,
            identity,
            process,
            wasm,
            marketplace_url: env::var("AIO_MARKETPLACE_URL").unwrap_or_else(|_| {
                "https://raw.githubusercontent.com/zjarlin/aio/main/marketplace/index.json"
                    .to_owned()
            }),
        };
        state.ensure_default_plugins().await?;
        state.reconcile_wasm().await?;
        state.reconcile_processes().await?;
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
        for mut plugin in plugins {
            let process = if plugin.runtime == crate::runtime::PluginRuntime::Process {
                Some(lifecycle::prepare_process(self, "default", &mut plugin).await?)
            } else {
                None
            };
            let wasm = if plugin.runtime == crate::runtime::PluginRuntime::WasmComponent {
                Some(lifecycle::prepare_wasm(self, "default", &mut plugin).await?)
            } else {
                None
            };
            let source_id = plugin.source_id.clone();
            let revision = plugin.revision.clone();
            if let Err(error) = self
                .store
                .activate("default", plugin, process.as_ref())
                .await
            {
                if let Some(process) = &process
                    && process.created
                {
                    let _ = self.process.stop(&process.instance_id).await;
                }
                let _ = lifecycle::cleanup_new_wasm(
                    self,
                    "default",
                    &source_id,
                    &revision,
                    wasm.as_ref(),
                );
                return Err(error.context("激活默认插件组合失败"));
            }
        }
        Ok(())
    }

    async fn reconcile_processes(&self) -> Result<()> {
        let targets = self.store.enabled_process_targets().await?;
        if targets.is_empty() {
            return Ok(());
        }
        self.process.health().await?;
        for target in targets {
            let instance = self
                .process
                .start(&target.tenant_id, &target.source_id, &target.revision)
                .await
                .with_context(|| {
                    format!(
                        "恢复 process 插件失败: tenant={} source={} revision={}",
                        target.tenant_id, target.source_id, target.revision
                    )
                })?;
            let validation = async {
                let pages = self.process.load_pages(&instance.endpoint).await?;
                self.repository.validate_pages(&target.revision, &pages)?;
                self.store
                    .verify_revision_pages(&target.revision_id, &pages)
                    .await
            }
            .await;
            if let Err(error) = validation {
                let _ = self.process.stop(&instance.instance_id).await;
                return Err(error.context("恢复 process 插件页面失败"));
            }
            if instance.created {
                self.store
                    .recover_process_instance(&target, &instance)
                    .await?;
            }
        }
        Ok(())
    }

    async fn reconcile_wasm(&self) -> Result<()> {
        for target in self.store.enabled_wasm_targets().await? {
            let activation = self
                .activate_wasm(
                    &target.tenant_id,
                    &target.source_id,
                    &target.revision,
                    &target.artifact,
                )
                .await
                .with_context(|| {
                    format!(
                        "恢复 Wasm Component 失败: tenant={} source={} revision={}",
                        target.tenant_id, target.source_id, target.revision
                    )
                })?;
            let validation = async {
                self.repository
                    .validate_pages(&target.revision, &activation.pages)?;
                self.store
                    .verify_revision_pages(&target.revision_id, &activation.pages)
                    .await
            }
            .await;
            if let Err(error) = validation {
                self.wasm
                    .deactivate(&target.tenant_id, &target.source_id, &target.revision)?;
                return Err(error.context("恢复 Wasm Component 页面失败"));
            }
            if activation.created {
                self.store.recover_wasm_instance(&target).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn activate_wasm(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
        artifact: &str,
    ) -> Result<wasm::WasmActivation> {
        let artifact = self.repository.artifact(revision, artifact)?;
        let manager = self.wasm.clone();
        let tenant_id = tenant_id.to_owned();
        let source_id = source_id.to_owned();
        let revision = revision.to_owned();
        tokio::task::spawn_blocking(move || {
            manager.activate(&tenant_id, &source_id, &revision, &artifact)
        })
        .await
        .context("等待 Wasm Component 实例化失败")?
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
