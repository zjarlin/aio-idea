use super::{
    RuntimeState, frontend_access::FrontendAccess, process::ProcessManager,
    repository::RepositoryInstaller, store::PluginStore, wasm::WasmManager,
};
use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
};

impl RuntimeState {
    pub(crate) async fn isolated_admin_test(
        identity: Arc<aio_plugin_identity_server::IdentityService>,
        database: &str,
        origin: &str,
        cache: &Path,
    ) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(database)
            .await?;
        let store = Arc::new(PluginStore::new(pool));
        store.migrate().await?;
        Ok(Self {
            store,
            repository: Arc::new(RepositoryInstaller::new(cache.to_path_buf())),
            identity,
            marketplace_url: String::new(),
            marketplace_syncing: Arc::new(Mutex::new(HashSet::from([String::new()]))),
            activation_locks: Arc::new(Mutex::new(HashMap::new())),
            publication_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            frontend: Arc::new(FrontendAccess::new(origin)?),
            process: Arc::new(ProcessManager::new()?),
            wasm: Arc::new(WasmManager::new()?),
            components: None,
        })
    }
}
