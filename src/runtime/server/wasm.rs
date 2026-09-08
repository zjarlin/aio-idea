use std::{
    collections::{HashMap, hash_map::Entry},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::{Context as _, Result, anyhow};
use serde::Deserialize;
use wasmtime::component::{Component, Instance, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use crate::runtime::PageDefinition;

const FUEL_PER_CALL: u64 = 10_000_000;
const MAX_CACHED_COMPONENTS: usize = 16;
const MAX_WASM_MEMORY_BYTES: usize = 128 * 1024 * 1024;
const MAX_WASM_TABLE_ELEMENTS: usize = 100_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct InstanceKey {
    tenant_id: String,
    source_id: String,
    revision: String,
}

impl InstanceKey {
    fn new(tenant_id: &str, source_id: &str, revision: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_owned(),
            source_id: source_id.to_owned(),
            revision: revision.to_owned(),
        }
    }
}

struct StoreState {
    limits: StoreLimits,
}

struct TenantInstance {
    store: Store<StoreState>,
    instance: Instance,
}

pub struct WasmActivation {
    pub pages: Vec<PageDefinition>,
    pub created: bool,
}

pub struct WasmManager {
    engine: Engine,
    components: Mutex<HashMap<PathBuf, Arc<Component>>>,
    instances: Mutex<HashMap<InstanceKey, Arc<Mutex<TenantInstance>>>>,
}

impl WasmManager {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true).consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|error| anyhow!("创建 Wasm Component 引擎失败: {error:#}"))?;
        Ok(Self {
            engine,
            components: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
        })
    }

    pub fn activate(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
        artifact: &Path,
    ) -> Result<WasmActivation> {
        let key = InstanceKey::new(tenant_id, source_id, revision);
        if let Some(instance) = self.instances()?.get(&key).cloned() {
            return Ok(WasmActivation {
                pages: call_definition(&instance)?,
                created: false,
            });
        }
        let candidate = Arc::new(Mutex::new(self.instantiate(artifact)?));
        let (instance, created) = {
            let mut instances = self.instances()?;
            match instances.entry(key.clone()) {
                Entry::Occupied(entry) => (entry.get().clone(), false),
                Entry::Vacant(entry) => (entry.insert(candidate).clone(), true),
            }
        };
        match call_definition(&instance) {
            Ok(pages) => Ok(WasmActivation { pages, created }),
            Err(error) => {
                if created {
                    self.instances()?.remove(&key);
                }
                Err(error)
            }
        }
    }

    pub fn deactivate(&self, tenant_id: &str, source_id: &str, revision: &str) -> Result<bool> {
        let key = InstanceKey::new(tenant_id, source_id, revision);
        Ok(self.instances()?.remove(&key).is_some())
    }

    pub fn handle(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
        request: String,
    ) -> Result<ComponentResponse> {
        let key = InstanceKey::new(tenant_id, source_id, revision);
        let instance = self
            .instances()?
            .get(&key)
            .cloned()
            .context("当前租户的 Wasm Component 实例未激活")?;
        call_handler(&instance, request)
    }

    fn instantiate(&self, artifact: &Path) -> Result<TenantInstance> {
        let component = self.component(artifact)?;
        let linker = Linker::new(&self.engine);
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_WASM_MEMORY_BYTES)
            .table_elements(MAX_WASM_TABLE_ELEMENTS)
            .instances(128)
            .tables(32)
            .memories(8)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&self.engine, StoreState { limits });
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(FUEL_PER_CALL)
            .map_err(|error| anyhow!("设置 Wasm fuel 失败: {error:#}"))?;
        let instance = linker
            .instantiate(&mut store, component.as_ref())
            .map_err(|error| anyhow!("实例化 Wasm Component 失败: {error:#}"))?;
        Ok(TenantInstance { store, instance })
    }

    fn component(&self, artifact: &Path) -> Result<Arc<Component>> {
        if let Some(component) = self.components()?.get(artifact).cloned() {
            return Ok(component);
        }
        let component = Arc::new(Component::from_file(&self.engine, artifact).map_err(
            |error| anyhow!("编译 Wasm Component 失败 {}: {error:#}", artifact.display()),
        )?);
        let mut components = self.components()?;
        if let Some(cached) = components.get(artifact) {
            return Ok(cached.clone());
        }
        if components.len() >= MAX_CACHED_COMPONENTS
            && let Some(stale) = components.keys().next().cloned()
        {
            components.remove(&stale);
        }
        Ok(components
            .entry(artifact.to_path_buf())
            .or_insert_with(|| component)
            .clone())
    }

    fn components(&self) -> Result<MutexGuard<'_, HashMap<PathBuf, Arc<Component>>>> {
        self.components
            .lock()
            .map_err(|_| anyhow!("Wasm Component 缓存锁已损坏"))
    }

    fn instances(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<InstanceKey, Arc<Mutex<TenantInstance>>>>> {
        self.instances
            .lock()
            .map_err(|_| anyhow!("Wasm Component 实例锁已损坏"))
    }

    #[cfg(test)]
    fn active_instances(&self) -> Result<usize> {
        Ok(self.instances()?.len())
    }
}

fn call_definition(instance: &Arc<Mutex<TenantInstance>>) -> Result<Vec<PageDefinition>> {
    let mut instance = instance
        .lock()
        .map_err(|_| anyhow!("Wasm 租户实例锁已损坏"))?;
    instance
        .store
        .set_fuel(FUEL_PER_CALL)
        .map_err(|error| anyhow!("重置 Wasm fuel 失败: {error:#}"))?;
    let component = instance.instance;
    let handle = component
        .get_typed_func::<(), (String,)>(&mut instance.store, "definition")
        .map_err(|error| anyhow!("Wasm Component 缺少 definition 导出: {error:#}"))?;
    let (json,) = handle
        .call(&mut instance.store, ())
        .map_err(|error| anyhow!("调用 Wasm Component definition 失败: {error:#}"))?;
    serde_json::from_str(&json)
        .map_err(|error| anyhow!("解析 Wasm Component PageDefinition 失败: {error}"))
}

fn call_handler(
    instance: &Arc<Mutex<TenantInstance>>,
    request: String,
) -> Result<ComponentResponse> {
    let mut instance = instance
        .lock()
        .map_err(|_| anyhow!("Wasm 租户实例锁已损坏"))?;
    instance
        .store
        .set_fuel(FUEL_PER_CALL)
        .map_err(|error| anyhow!("重置 Wasm fuel 失败: {error:#}"))?;
    let component = instance.instance;
    let handle = component
        .get_typed_func::<(String,), (String,)>(&mut instance.store, "handle")
        .map_err(|error| anyhow!("Wasm Component 缺少 handle 导出: {error:#}"))?;
    let (json,) = handle
        .call(&mut instance.store, (request,))
        .map_err(|error| anyhow!("调用 Wasm Component handle 失败: {error:#}"))?;
    serde_json::from_str(&json).map_err(|error| anyhow!("解析 Wasm Component 响应失败: {error}"))
}

#[derive(Debug, Deserialize)]
pub struct ComponentResponse {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_instance_keys_by_tenant_source_and_revision() {
        let base = InstanceKey::new("tenant-a", "source-a", &"a".repeat(40));
        assert_eq!(
            base,
            InstanceKey::new("tenant-a", "source-a", &"a".repeat(40))
        );
        assert_ne!(
            base,
            InstanceKey::new("tenant-b", "source-a", &"a".repeat(40))
        );
        assert_ne!(
            base,
            InstanceKey::new("tenant-a", "source-b", &"a".repeat(40))
        );
        assert_ne!(
            base,
            InstanceKey::new("tenant-a", "source-a", &"b".repeat(40))
        );
    }

    #[test]
    fn starts_without_active_instances() -> Result<()> {
        let manager = WasmManager::new()?;
        assert_eq!(manager.active_instances()?, 0);
        Ok(())
    }

    #[test]
    #[ignore = "需要 AIO_TEST_WASM_COMPONENT 指向真实插件产物"]
    fn activates_reuses_and_destroys_tenant_instances() -> Result<()> {
        let artifact = std::env::var("AIO_TEST_WASM_COMPONENT")?;
        let manager = WasmManager::new()?;
        let revision = "a".repeat(40);
        let first = manager.activate("tenant-a", "source-a", &revision, Path::new(&artifact))?;
        assert!(first.created);
        assert!(!first.pages.is_empty());

        let reused = manager.activate("tenant-a", "source-a", &revision, Path::new(&artifact))?;
        assert!(!reused.created);
        manager.activate("tenant-b", "source-a", &revision, Path::new(&artifact))?;
        assert_eq!(manager.active_instances()?, 2);

        let response = manager.handle(
            "tenant-a",
            "source-a",
            &revision,
            serde_json::json!({
                "method": "POST",
                "path": "/echo",
                "query": null,
                "body": "tenant scoped",
                "tenant_id": "tenant-a",
                "user_id": "user-a"
            })
            .to_string(),
        )?;
        assert_eq!(response.status, 200);
        assert!(response.body.contains("tenant-a"));

        let action = |tenant_id: &str| {
            manager.handle(
                tenant_id,
                "source-a",
                &revision,
                serde_json::json!({
                    "kind": "page_action",
                    "page_id": "ts-counter",
                    "action_id": "increment",
                    "tenant_id": tenant_id,
                    "user_id": "user-a"
                })
                .to_string(),
            )
        };
        let content = |response: ComponentResponse| -> Result<String> {
            let result = serde_json::from_str::<crate::runtime::PageActionResult>(&response.body)?;
            let crate::runtime::PageBody::Actions { content, .. } = result.body else {
                anyhow::bail!("页面动作没有返回 actions 页面体");
            };
            Ok(content)
        };
        assert_eq!(content(action("tenant-a")?)?, "计数：1");
        assert_eq!(content(action("tenant-b")?)?, "计数：1");
        assert_eq!(content(action("tenant-a")?)?, "计数：2");

        assert!(manager.deactivate("tenant-a", "source-a", &revision)?);
        assert_eq!(manager.active_instances()?, 1);
        assert!(
            manager
                .handle("tenant-a", "source-a", &revision, "{}".to_owned())
                .is_err()
        );
        Ok(())
    }
}
