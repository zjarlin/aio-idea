use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use wasmtime::{
    Config, Engine, Store,
    component::{Component, Linker},
};

use crate::runtime::PageDefinition;

const MAX_CACHED_COMPONENTS: usize = 16;

struct ComponentRuntime {
    engine: Engine,
    components: Mutex<HashMap<PathBuf, Arc<Component>>>,
}

impl ComponentRuntime {
    fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true).consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|error| anyhow!("创建 Wasm Component 引擎失败: {error:#}"))?;
        Ok(Self {
            engine,
            components: Mutex::new(HashMap::new()),
        })
    }

    fn component(&self, artifact: &Path) -> Result<Arc<Component>> {
        if let Some(component) = self
            .components
            .lock()
            .map_err(|_| anyhow!("Wasm Component 缓存锁已损坏"))?
            .get(artifact)
            .cloned()
        {
            return Ok(component);
        }
        let component = Arc::new(Component::from_file(&self.engine, artifact).map_err(
            |error| anyhow!("编译 Wasm Component 失败 {}: {error:#}", artifact.display()),
        )?);
        let mut components = self
            .components
            .lock()
            .map_err(|_| anyhow!("Wasm Component 缓存锁已损坏"))?;
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
}

fn runtime() -> Result<&'static ComponentRuntime> {
    static RUNTIME: OnceLock<Result<ComponentRuntime>> = OnceLock::new();
    RUNTIME
        .get_or_init(ComponentRuntime::new)
        .as_ref()
        .map_err(|error| anyhow!("{error:#}"))
}

pub fn load_pages(artifact: &Path) -> Result<Vec<PageDefinition>> {
    let runtime = runtime()?;
    let component = runtime.component(artifact)?;
    let linker = Linker::new(&runtime.engine);
    let mut store = Store::new(&runtime.engine, ());
    store
        .set_fuel(10_000_000)
        .map_err(|error| anyhow!("设置 Wasm fuel 失败: {error:#}"))?;
    let instance = linker
        .instantiate(&mut store, component.as_ref())
        .map_err(|error| anyhow!("实例化 Wasm Component 失败: {error:#}"))?;
    let definition = instance
        .get_typed_func::<(), (String,)>(&mut store, "definition")
        .map_err(|error| anyhow!("Wasm Component 缺少 definition 导出: {error:#}"))?;
    let (json,) = definition
        .call(&mut store, ())
        .map_err(|error| anyhow!("调用 Wasm Component definition 失败: {error:#}"))?;
    serde_json::from_str(&json)
        .map_err(|error| anyhow!("解析 Wasm Component PageDefinition 失败: {error}"))
}

#[derive(Debug, Deserialize)]
pub struct ComponentResponse {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

pub fn handle(artifact: &Path, request: String) -> Result<ComponentResponse> {
    let runtime = runtime()?;
    let component = runtime.component(artifact)?;
    let linker = Linker::new(&runtime.engine);
    let mut store = Store::new(&runtime.engine, ());
    store
        .set_fuel(10_000_000)
        .map_err(|error| anyhow!("设置 Wasm fuel 失败: {error:#}"))?;
    let instance = linker
        .instantiate(&mut store, component.as_ref())
        .map_err(|error| anyhow!("实例化 Wasm Component 失败: {error:#}"))?;
    let handler = instance
        .get_typed_func::<(String,), (String,)>(&mut store, "handle")
        .map_err(|error| anyhow!("Wasm Component 缺少 handle 导出: {error:#}"))?;
    let (json,) = handler
        .call(&mut store, (request,))
        .map_err(|error| anyhow!("调用 Wasm Component handle 失败: {error:#}"))?;
    serde_json::from_str(&json).map_err(|error| anyhow!("解析 Wasm Component 响应失败: {error}"))
}
