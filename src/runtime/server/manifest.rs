use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::runtime::PluginRuntime;

#[derive(Debug, Deserialize, Serialize)]
pub struct RepositoryManifest {
    pub plugin: PluginManifest,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PluginManifest {
    pub runtime: RuntimeManifest,
    #[serde(default)]
    pub capabilities: CapabilityManifest,
    #[serde(default)]
    pub subplugins: Vec<SubpluginManifest>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RuntimeManifest {
    pub kind: PluginRuntime,
    pub artifact: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct CapabilityManifest {
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub filesystem: Vec<String>,
    #[serde(default)]
    pub database: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SubpluginManifest {
    pub id: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub pages: Vec<String>,
    #[serde(default)]
    pub routes: Vec<String>,
    #[serde(default)]
    pub account_actions: Vec<String>,
}

pub fn parse(source: &str) -> Result<RepositoryManifest> {
    let manifest =
        toml::from_str::<RepositoryManifest>(source).context("解析插件运行时清单失败")?;
    validate(&manifest)?;
    Ok(manifest)
}

fn validate(manifest: &RepositoryManifest) -> Result<()> {
    ensure!(
        !manifest.plugin.runtime.artifact.trim().is_empty(),
        "插件 artifact 不能为空"
    );
    if manifest.plugin.runtime.kind == PluginRuntime::WasmComponent {
        ensure!(
            manifest.plugin.capabilities.network.is_empty()
                && manifest.plugin.capabilities.filesystem.is_empty()
                && !manifest.plugin.capabilities.database,
            "当前 Wasm Component 宿主未授予网络、文件系统或数据库能力"
        );
    }
    let plugins = manifest
        .plugin
        .subplugins
        .iter()
        .map(|plugin| (plugin.id.as_str(), plugin))
        .collect::<HashMap<_, _>>();
    ensure!(
        plugins.len() == manifest.plugin.subplugins.len(),
        "子插件 id 重复"
    );
    let mut pages = HashSet::new();
    for plugin in &manifest.plugin.subplugins {
        ensure!(!plugin.id.trim().is_empty(), "子插件 id 不能为空");
        for page in &plugin.pages {
            ensure!(pages.insert(page.as_str()), "子插件页面声明重复: {page}");
        }
        for dependency in &plugin.dependencies {
            ensure!(
                plugins.contains_key(dependency.as_str()),
                "子插件 {} 依赖不存在: {dependency}",
                plugin.id
            );
        }
    }
    let mut states = HashMap::<&str, VisitState>::new();
    for id in plugins.keys() {
        visit(id, &plugins, &mut states)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Visited,
}

fn visit<'a>(
    id: &'a str,
    plugins: &HashMap<&'a str, &'a SubpluginManifest>,
    states: &mut HashMap<&'a str, VisitState>,
) -> Result<()> {
    match states.get(id) {
        Some(VisitState::Visiting) => anyhow::bail!("子插件依赖存在循环: {id}"),
        Some(VisitState::Visited) => return Ok(()),
        None => {}
    }
    states.insert(id, VisitState::Visiting);
    for dependency in &plugins[id].dependencies {
        visit(dependency, plugins, states)?;
    }
    states.insert(id, VisitState::Visited);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNTIME: &str = r#"
[plugin.runtime]
kind = "wasm-component"
artifact = "dist/plugin.wasm"
"#;

    #[test]
    fn accepts_parent_with_ordered_subplugins() -> Result<()> {
        let manifest = parse(&format!(
            "{RUNTIME}\n[[plugin.subplugins]]\nid='profile'\npages=['profile']\n[[plugin.subplugins]]\nid='account'\ndependencies=['profile']\naccount_actions=['profile']"
        ))?;
        assert_eq!(manifest.plugin.subplugins.len(), 2);
        Ok(())
    }

    #[test]
    fn rejects_missing_dependency() {
        let error = parse(&format!(
            "{RUNTIME}\n[[plugin.subplugins]]\nid='account'\ndependencies=['missing']"
        ))
        .unwrap_err();
        assert!(error.to_string().contains("依赖不存在"));
    }

    #[test]
    fn rejects_dependency_cycle() {
        let error = parse(&format!(
            "{RUNTIME}\n[[plugin.subplugins]]\nid='a'\ndependencies=['b']\n[[plugin.subplugins]]\nid='b'\ndependencies=['a']"
        ))
        .unwrap_err();
        assert!(error.to_string().contains("循环"));
    }

    #[test]
    fn rejects_duplicate_page_declaration() {
        let error = parse(&format!(
            "{RUNTIME}\n[[plugin.subplugins]]\nid='a'\npages=['home']\n[[plugin.subplugins]]\nid='b'\npages=['home']"
        ))
        .unwrap_err();
        assert!(error.to_string().contains("页面声明重复"));
    }

    #[test]
    fn rejects_ungranted_wasm_capability() {
        let error = parse(&format!(
            "{RUNTIME}\n[plugin.capabilities]\nnetwork=['api.example.com']"
        ))
        .unwrap_err();
        assert!(error.to_string().contains("未授予"));
    }
}
