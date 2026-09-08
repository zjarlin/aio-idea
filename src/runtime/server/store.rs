use anyhow::{Context as _, Result, ensure};
use serde_json::Value;
use sqlx::{PgPool, Row};

use super::repository::DiscoveredPlugin;
use crate::runtime::{
    InstalledPluginView, PageDefinition, PluginRuntime, PluginState, RuntimeCatalog, TenantView,
    UserView,
};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS plugin_sources (
    id TEXT PRIMARY KEY,
    git TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS plugin_revisions (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES plugin_sources(id) ON DELETE CASCADE,
    revision TEXT NOT NULL,
    runtime TEXT NOT NULL,
    manifest JSONB NOT NULL,
    pages JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(source_id, revision)
);
CREATE TABLE IF NOT EXISTS plugin_runtime_instances (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    revision_id TEXT NOT NULL REFERENCES plugin_revisions(id),
    state TEXT NOT NULL,
    started_at TIMESTAMPTZ,
    stopped_at TIMESTAMPTZ
);
CREATE TABLE IF NOT EXISTS tenant_plugin_bindings (
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL REFERENCES plugin_sources(id) ON DELETE CASCADE,
    revision_id TEXT NOT NULL REFERENCES plugin_revisions(id),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(tenant_id, source_id)
);
CREATE TABLE IF NOT EXISTS plugin_lifecycle_events (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    revision_id TEXT,
    lifecycle TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS plugin_registries (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    source TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    UNIQUE(tenant_id, source)
);
"#;

pub struct PluginStore {
    pool: PgPool,
}

impl PluginStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::raw_sql(SCHEMA)
            .execute(&self.pool)
            .await
            .context("创建插件运行时表失败")?;
        Ok(())
    }

    pub async fn has_plugins(&self, tenant_id: &str) -> Result<bool> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM tenant_plugin_bindings WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn activate(&self, tenant_id: &str, plugin: DiscoveredPlugin) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO plugin_sources (id, git) VALUES ($1, $2) ON CONFLICT (git) DO UPDATE SET git = EXCLUDED.git",
        )
        .bind(&plugin.source_id)
        .bind(&plugin.git)
        .execute(&mut *transaction)
        .await?;
        let source_id =
            sqlx::query_scalar::<_, String>("SELECT id FROM plugin_sources WHERE git = $1")
                .bind(&plugin.git)
                .fetch_one(&mut *transaction)
                .await?;
        let revision_id = format!("{source_id}:{}", plugin.revision);
        sqlx::query(
            "INSERT INTO plugin_revisions (id, source_id, revision, runtime, manifest, pages) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (source_id, revision) DO UPDATE SET manifest = EXCLUDED.manifest, pages = EXCLUDED.pages",
        )
        .bind(&revision_id)
        .bind(&source_id)
        .bind(&plugin.revision)
        .bind(runtime_name(plugin.runtime))
        .bind(&plugin.manifest)
        .bind(serde_json::to_value(&plugin.pages)?)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO tenant_plugin_bindings (tenant_id, source_id, revision_id, enabled) VALUES ($1, $2, $3, TRUE) ON CONFLICT (tenant_id, source_id) DO UPDATE SET revision_id = EXCLUDED.revision_id, enabled = TRUE, updated_at = now()",
        )
        .bind(tenant_id)
        .bind(&source_id)
        .bind(&revision_id)
        .execute(&mut *transaction)
        .await?;
        stop_instances(&mut transaction, tenant_id, &source_id).await?;
        start_instance(&mut transaction, tenant_id, &revision_id).await?;
        record_event(
            &mut transaction,
            tenant_id,
            &source_id,
            Some(&revision_id),
            "activate",
            "健康检查通过并原子激活",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn catalog(
        &self,
        tenant_id: &str,
        tenant_label: &str,
        user: UserView,
    ) -> Result<RuntimeCatalog> {
        let rows = sqlx::query(
            "SELECT sources.id, sources.git, revisions.revision, revisions.runtime, revisions.pages, bindings.enabled FROM tenant_plugin_bindings bindings JOIN plugin_sources sources ON sources.id = bindings.source_id JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id WHERE bindings.tenant_id = $1 ORDER BY sources.git",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        let mut pages = Vec::new();
        let mut plugins = Vec::new();
        for row in rows {
            let enabled: bool = row.try_get("enabled")?;
            let value: Value = row.try_get("pages")?;
            if enabled {
                pages.extend(serde_json::from_value::<Vec<PageDefinition>>(value)?);
            }
            plugins.push(InstalledPluginView {
                source_id: row.try_get("id")?,
                git: row.try_get("git")?,
                revision: row.try_get("revision")?,
                runtime: parse_runtime(row.try_get("runtime")?)?,
                state: if enabled {
                    PluginState::Active
                } else {
                    PluginState::Disabled
                },
            });
        }
        ensure_unique_pages(&pages)?;
        Ok(RuntimeCatalog {
            tenant: TenantView {
                id: tenant_id.to_owned(),
                label: tenant_label.to_owned(),
            },
            user,
            pages,
            plugins,
        })
    }

    pub async fn set_enabled(&self, tenant_id: &str, source_id: &str, enabled: bool) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        let revision_id = sqlx::query_scalar::<_, String>("UPDATE tenant_plugin_bindings SET enabled = $3, updated_at = now() WHERE tenant_id = $1 AND source_id = $2 RETURNING revision_id")
            .bind(tenant_id).bind(source_id).bind(enabled).fetch_optional(&mut *transaction).await?
            .context("插件绑定不存在")?;
        stop_instances(&mut transaction, tenant_id, source_id).await?;
        if enabled {
            start_instance(&mut transaction, tenant_id, &revision_id).await?;
        }
        record_event(
            &mut transaction,
            tenant_id,
            source_id,
            Some(&revision_id),
            if enabled { "activate" } else { "deactivate" },
            "运行时状态已切换",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn uninstall(&self, tenant_id: &str, source_id: &str) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        stop_instances(&mut transaction, tenant_id, source_id).await?;
        let result = sqlx::query(
            "DELETE FROM tenant_plugin_bindings WHERE tenant_id = $1 AND source_id = $2",
        )
        .bind(tenant_id)
        .bind(source_id)
        .execute(&mut *transaction)
        .await?;
        ensure!(result.rows_affected() == 1, "插件绑定不存在");
        record_event(
            &mut transaction,
            tenant_id,
            source_id,
            None,
            "uninstall",
            "租户组合已移除插件",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn rollback(&self, tenant_id: &str, source_id: &str) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        let revision_id = sqlx::query_scalar::<_, String>(
            "SELECT revisions.id FROM plugin_revisions revisions JOIN tenant_plugin_bindings bindings ON bindings.source_id = revisions.source_id WHERE bindings.tenant_id = $1 AND bindings.source_id = $2 AND revisions.id <> bindings.revision_id ORDER BY revisions.created_at DESC LIMIT 1",
        )
        .bind(tenant_id).bind(source_id).fetch_optional(&mut *transaction).await?
        .context("没有可回滚的历史版本")?;
        stop_instances(&mut transaction, tenant_id, source_id).await?;
        sqlx::query("UPDATE tenant_plugin_bindings SET revision_id = $3, enabled = TRUE, updated_at = now() WHERE tenant_id = $1 AND source_id = $2")
            .bind(tenant_id).bind(source_id).bind(&revision_id).execute(&mut *transaction).await?;
        start_instance(&mut transaction, tenant_id, &revision_id).await?;
        record_event(
            &mut transaction,
            tenant_id,
            source_id,
            Some(&revision_id),
            "rollback",
            "已切回上一健康版本",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn registry_sources(&self, tenant_id: &str) -> Result<Vec<String>> {
        sqlx::query_scalar("SELECT source FROM plugin_registries WHERE tenant_id = $1 AND enabled = TRUE ORDER BY source")
            .bind(tenant_id).fetch_all(&self.pool).await.map_err(Into::into)
    }

    pub async fn add_registry(&self, tenant_id: &str, source: &str) -> Result<()> {
        sqlx::query("INSERT INTO plugin_registries (id, tenant_id, source) VALUES ($1, $2, $3) ON CONFLICT (tenant_id, source) DO UPDATE SET enabled = TRUE")
            .bind(uuid::Uuid::new_v4().to_string()).bind(tenant_id).bind(source).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn active_component(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Option<(String, String)>> {
        sqlx::query_as(
            "SELECT revisions.revision, revisions.manifest->'runtime'->>'artifact' FROM tenant_plugin_bindings bindings JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id WHERE bindings.tenant_id = $1 AND bindings.source_id = $2 AND bindings.enabled = TRUE AND revisions.runtime = 'wasm-component'",
        )
        .bind(tenant_id)
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }
}

fn ensure_unique_pages(pages: &[PageDefinition]) -> Result<()> {
    let mut ids = std::collections::HashSet::new();
    for page in pages {
        ensure!(!page.id.trim().is_empty(), "运行时页面 id 不能为空");
        ensure!(
            ids.insert(page.id.as_str()),
            "运行时页面 id 重复: {}",
            page.id
        );
    }
    Ok(())
}

async fn record_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    source_id: &str,
    revision_id: Option<&str>,
    lifecycle: &str,
    detail: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO plugin_lifecycle_events (id, tenant_id, source_id, revision_id, lifecycle, detail) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(uuid::Uuid::new_v4().to_string()).bind(tenant_id).bind(source_id)
        .bind(revision_id).bind(lifecycle).bind(detail).execute(&mut **transaction).await?;
    Ok(())
}

async fn stop_instances(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    source_id: &str,
) -> Result<()> {
    sqlx::query("UPDATE plugin_runtime_instances SET state = 'stopped', stopped_at = now() WHERE tenant_id = $1 AND state = 'active' AND revision_id IN (SELECT id FROM plugin_revisions WHERE source_id = $2)")
        .bind(tenant_id).bind(source_id).execute(&mut **transaction).await?;
    Ok(())
}

async fn start_instance(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    revision_id: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO plugin_runtime_instances (id, tenant_id, revision_id, state, started_at) VALUES ($1, $2, $3, 'active', now())")
        .bind(uuid::Uuid::new_v4().to_string()).bind(tenant_id).bind(revision_id)
        .execute(&mut **transaction).await?;
    Ok(())
}

fn runtime_name(runtime: PluginRuntime) -> &'static str {
    match runtime {
        PluginRuntime::PageDefinition => "page-definition",
        PluginRuntime::WasmComponent => "wasm-component",
        PluginRuntime::Process => "process",
        PluginRuntime::RustSource => "rust-source",
    }
}

fn parse_runtime(value: String) -> Result<PluginRuntime> {
    match value.as_str() {
        "page-definition" => Ok(PluginRuntime::PageDefinition),
        "wasm-component" => Ok(PluginRuntime::WasmComponent),
        "process" => Ok(PluginRuntime::Process),
        "rust-source" => Ok(PluginRuntime::RustSource),
        _ => anyhow::bail!("未知插件运行时: {value}"),
    }
}
