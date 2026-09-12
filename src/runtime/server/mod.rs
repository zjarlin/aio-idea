mod activation_store;
#[cfg(test)]
mod admin_test_support;
mod delivery;
mod frontend_access;
#[cfg(test)]
mod frontend_browser_tests;
mod frontend_delivery;
mod frontend_document;
#[cfg(test)]
mod frontend_http_tests;
mod frontend_model;
mod frontend_package;
mod frontend_routes;
#[cfg(test)]
mod frontend_tests;
mod http_error;
mod installation;
mod lifecycle;
mod management;
mod marketplace_store;
mod package_repository;
mod package_store;
mod page_state;
mod process;
mod publication;
#[cfg(test)]
mod publication_http_tests;
mod publication_validation;
mod publisher_store;
mod remote_access;
mod repository;
mod request_context;
mod routes;
mod service_dispatch;
mod source_migration;
mod store;
mod supervisor;
mod wasm;

use std::{
    collections::{HashMap, HashSet},
    env,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};

use anyhow::{Context as _, Result};
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;

use crate::runtime::PublishState;

pub use routes::router;
pub use supervisor::run as run_supervisor;

#[derive(Clone)]
pub struct RuntimeState {
    pub store: Arc<store::PluginStore>,
    pub repository: Arc<repository::RepositoryInstaller>,
    pub identity: Arc<aio_plugin_identity_server::IdentityService>,
    pub marketplace_url: String,
    marketplace_syncing: Arc<Mutex<HashSet<String>>>,
    activation_locks: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
    publication_slots: Arc<tokio::sync::Semaphore>,
    frontend: Arc<frontend_access::FrontendAccess>,
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
            marketplace_syncing: Arc::new(Mutex::new(HashSet::new())),
            activation_locks: Arc::new(Mutex::new(HashMap::new())),
            publication_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            frontend: Arc::new(frontend_access::FrontendAccess::new(
                &env::var("AIO_PUBLIC_ORIGIN")
                    .unwrap_or_else(|_| "https://aio.addzero.site".to_owned()),
            )?),
            marketplace_url: env::var("AIO_MARKETPLACE_URL")
                .unwrap_or_else(|_| "https://github.com/zjarlin/aio-platform.git".to_owned()),
        };
        state.sync_marketplace_sources([state.marketplace_url.clone()]);
        state.ensure_default_plugins().await?;
        state.reconcile_wasm().await?;
        state.reconcile_processes().await?;
        state.resume_published_jobs().await?;
        delivery::start(state.clone());
        Ok(state)
    }

    pub(super) fn activation_lock(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Arc<tokio::sync::Mutex<()>>> {
        let key = format!("{tenant_id}\0{source_id}");
        let mut locks = self
            .activation_locks
            .lock()
            .map_err(|_| anyhow::anyhow!("插件生命周期锁已损坏"))?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        Ok(lock)
    }

    pub(super) fn sync_marketplace_sources(&self, sources: impl IntoIterator<Item = String>) {
        for source in sources {
            let should_start = self
                .marketplace_syncing
                .lock()
                .map(|mut syncing| syncing.insert(source.clone()))
                .unwrap_or(false);
            if !should_start {
                continue;
            }
            let repository = self.repository.clone();
            let store = self.store.clone();
            let syncing = self.marketplace_syncing.clone();
            tokio::spawn(async move {
                let result = match repository.registry(&source).await {
                    Ok(entries) => store.replace_marketplace_entries(&source, &entries).await,
                    Err(error) => {
                        store
                            .record_marketplace_sync_failure(&source, &format!("{error:#}"))
                            .await
                    }
                };
                if result.is_err() {
                    let _ = store
                        .record_marketplace_sync_failure(&source, "写入市场索引缓存失败")
                        .await;
                }
                if let Ok(mut active) = syncing.lock() {
                    active.remove(&source);
                }
            });
        }
    }

    fn start_publish_job(&self, job: publisher_store::PublishJob) {
        if job.state != PublishState::Queued {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            let Ok(_permit) = state.publication_slots.clone().acquire_owned().await else {
                return;
            };
            let claimed = match state.store.claim_publish_job(&job.id).await {
                Ok(claimed) => claimed,
                Err(error) => {
                    eprintln!("领取插件发布任务失败: {}", error);
                    return;
                }
            };
            if !claimed {
                return;
            }
            let _ = state
                .store
                .record_lifecycle_event(
                    &job.tenant_id,
                    &job.source_id,
                    None,
                    "publish-verify",
                    "正在校验运行时 artifact、页面定义和健康检查",
                )
                .await;
            let result = async {
                state.restore_package_cache(&job.revision).await?;
                let discovered = state
                    .repository
                    .validate_published(&job.git, &job.revision)
                    .await?;
                let publication = state
                    .repository
                    .published_marketplace_entry(&job.git, &job.revision)?;
                let activated = installation::activate(
                    &state,
                    &job.tenant_id,
                    discovered,
                    "已校验二进制插件包内容摘要、能力声明和运行时协议",
                    Some(&publication),
                )
                .await?;
                Ok::<_, anyhow::Error>(activated)
            }
            .await;
            let (publish_state, lifecycle, detail, page_count) = match result {
                Ok(activated) => {
                    let detail = format!(
                        "版本 {} 已在线激活，共 {} 个页面",
                        activated.revision, activated.page_count
                    );
                    (
                        PublishState::Active,
                        "publish-active",
                        detail,
                        Some(activated.page_count),
                    )
                }
                Err(error) => (
                    PublishState::Failed,
                    "publish-failed",
                    format!("后台验证或激活失败，已保留上一活动版本: {error:#}"),
                    None,
                ),
            };
            if let Err(error) = state
                .store
                .finish_publish_job(&job.id, publish_state, &detail, page_count)
                .await
            {
                eprintln!("记录插件发布任务结果失败: {}", error);
            }
            if let Err(error) = state
                .store
                .record_lifecycle_event(&job.tenant_id, &job.source_id, None, lifecycle, &detail)
                .await
            {
                eprintln!("记录插件发布生命周期失败: {}", error);
            }
        });
    }

    async fn resume_published_jobs(&self) -> Result<()> {
        for job in self.store.resume_publish_jobs().await? {
            self.start_publish_job(job);
        }
        Ok(())
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
                .activate("default", plugin, process.as_ref(), None)
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
        self.store.stop_orphan_process_records().await?;
        let targets = self.store.enabled_process_targets().await?;
        for target in &targets {
            self.restore_package_cache(&target.revision).await?;
        }
        self.process.health().await?;
        self.process
            .reconcile(
                targets
                    .iter()
                    .map(|target| supervisor::StartProcessRequest {
                        tenant_id: target.tenant_id.clone(),
                        source_id: target.source_id.clone(),
                        revision: target.revision.clone(),
                    })
                    .collect(),
            )
            .await?;
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
            if instance.created
                && let Err(error) = self
                    .store
                    .recover_process_instance(&target, &instance)
                    .await
            {
                let _ = self.process.stop(&instance.instance_id).await;
                return Err(error.context("记录恢复的 process 插件实例失败"));
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
        self.restore_package_cache(revision).await?;
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
