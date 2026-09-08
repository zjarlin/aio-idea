use std::path::Path;

use anyhow::{Result, anyhow};
use serde::Deserialize;
use wasmtime::{
    Config, Engine, Store,
    component::{Component, Linker},
};

use crate::runtime::PageDefinition;

pub fn load_pages(artifact: &Path) -> Result<Vec<PageDefinition>> {
    let mut config = Config::new();
    config.wasm_component_model(true).consume_fuel(true);
    let engine =
        Engine::new(&config).map_err(|error| anyhow!("创建 Wasm Component 引擎失败: {error:#}"))?;
    let component = Component::from_file(&engine, artifact)
        .map_err(|error| anyhow!("编译 Wasm Component 失败 {}: {error:#}", artifact.display()))?;
    let linker = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    store
        .set_fuel(10_000_000)
        .map_err(|error| anyhow!("设置 Wasm fuel 失败: {error:#}"))?;
    let instance = linker
        .instantiate(&mut store, &component)
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
    let mut config = Config::new();
    config.wasm_component_model(true).consume_fuel(true);
    let engine =
        Engine::new(&config).map_err(|error| anyhow!("创建 Wasm Component 引擎失败: {error:#}"))?;
    let component = Component::from_file(&engine, artifact)
        .map_err(|error| anyhow!("编译 Wasm Component 失败 {}: {error:#}", artifact.display()))?;
    let linker = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    store
        .set_fuel(10_000_000)
        .map_err(|error| anyhow!("设置 Wasm fuel 失败: {error:#}"))?;
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|error| anyhow!("实例化 Wasm Component 失败: {error:#}"))?;
    let handler = instance
        .get_typed_func::<(String,), (String,)>(&mut store, "handle")
        .map_err(|error| anyhow!("Wasm Component 缺少 handle 导出: {error:#}"))?;
    let (json,) = handler
        .call(&mut store, (request,))
        .map_err(|error| anyhow!("调用 Wasm Component handle 失败: {error:#}"))?;
    serde_json::from_str(&json).map_err(|error| anyhow!("解析 Wasm Component 响应失败: {error}"))
}
