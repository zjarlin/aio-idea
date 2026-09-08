use serde::{Deserialize, Serialize};

#[cfg(any(feature = "web", feature = "desktop"))]
pub use az_plugin_manifest::PageBody;
pub use az_plugin_manifest::{PageDefinition, PluginRuntime};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCatalog {
    pub tenant: TenantView,
    pub user: UserView,
    pub pages: Vec<PageDefinition>,
    #[serde(default)]
    pub account_items: Vec<RuntimeAccountItem>,
    pub plugins: Vec<InstalledPluginView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TenantView {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserView {
    pub label: String,
    pub handle: String,
    pub initials: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAccountItem {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub page_id: String,
    #[serde(default)]
    pub required_permission: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstalledPluginView {
    pub source_id: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub state: PluginState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Active,
    Disabled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "server")]
pub struct InstallPluginRequest {
    pub git: String,
    pub rev: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "server")]
pub struct MarketplaceEntry {
    pub git: String,
    pub rev: String,
    pub title: String,
    pub summary: String,
    pub license: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub state: Option<PluginState>,
    #[serde(default)]
    pub active_revision: Option<String>,
    #[serde(default)]
    pub runtime: Option<PluginRuntime>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeResponse<T> {
    pub data: T,
}
