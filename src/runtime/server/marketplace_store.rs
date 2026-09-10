use anyhow::{Context as _, Result};
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::store::{PluginStore, parse_runtime, runtime_name};
use crate::runtime::MarketplaceEntry;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS marketplace_entries (
    source TEXT NOT NULL,
    git TEXT NOT NULL,
    rev TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    license TEXT NOT NULL,
    tags JSONB NOT NULL,
    runtime TEXT,
    capabilities JSONB NOT NULL,
    synced_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(source, git)
);
CREATE TABLE IF NOT EXISTS marketplace_registry_syncs (
    source TEXT PRIMARY KEY,
    last_success_at TIMESTAMPTZ,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS marketplace_entries_source_title_idx ON marketplace_entries(source, title);
"#;

pub(super) const PUBLISHED_REGISTRY_SOURCE: &str = "aio://published";

pub(super) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(SCHEMA)
        .execute(pool)
        .await
        .context("创建市场缓存表失败")?;
    Ok(())
}

pub(super) async fn upsert_published_marketplace_entry(
    transaction: &mut Transaction<'_, Postgres>,
    entry: &MarketplaceEntry,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO marketplace_entries (source, git, rev, title, summary, license, tags, runtime, capabilities, synced_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now()) ON CONFLICT (source, git) DO UPDATE SET rev = EXCLUDED.rev, title = EXCLUDED.title, summary = EXCLUDED.summary, license = EXCLUDED.license, tags = EXCLUDED.tags, runtime = EXCLUDED.runtime, capabilities = EXCLUDED.capabilities, synced_at = now()",
    )
    .bind(PUBLISHED_REGISTRY_SOURCE)
    .bind(&entry.git)
    .bind(&entry.rev)
    .bind(&entry.title)
    .bind(&entry.summary)
    .bind(&entry.license)
    .bind(serde_json::to_value(&entry.tags)?)
    .bind(entry.runtime.map(runtime_name))
    .bind(serde_json::to_value(&entry.capabilities)?)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

impl PluginStore {
    pub async fn replace_marketplace_entries(
        &self,
        source: &str,
        entries: &[MarketplaceEntry],
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM marketplace_entries WHERE source = $1")
            .bind(source)
            .execute(&mut *transaction)
            .await?;
        for entry in entries {
            sqlx::query(
                "INSERT INTO marketplace_entries (source, git, rev, title, summary, license, tags, runtime, capabilities) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(source)
            .bind(&entry.git)
            .bind(&entry.rev)
            .bind(&entry.title)
            .bind(&entry.summary)
            .bind(&entry.license)
            .bind(serde_json::to_value(&entry.tags)?)
            .bind(entry.runtime.map(runtime_name))
            .bind(serde_json::to_value(&entry.capabilities)?)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO marketplace_registry_syncs (source, last_success_at, last_error) VALUES ($1, now(), NULL) ON CONFLICT (source) DO UPDATE SET last_success_at = now(), last_error = NULL, updated_at = now()",
        )
        .bind(source)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn record_marketplace_sync_failure(&self, source: &str, error: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO marketplace_registry_syncs (source, last_error) VALUES ($1, $2) ON CONFLICT (source) DO UPDATE SET last_error = EXCLUDED.last_error, updated_at = now()",
        )
        .bind(source)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn marketplace_entries(&self, source: &str) -> Result<Vec<MarketplaceEntry>> {
        let rows = sqlx::query(
            "SELECT git, rev, title, summary, license, tags, runtime, capabilities FROM marketplace_entries WHERE source = $1 ORDER BY title, git",
        )
        .bind(source)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let runtime = row
                    .try_get::<Option<String>, _>("runtime")?
                    .map(parse_runtime)
                    .transpose()?;
                Ok(MarketplaceEntry {
                    git: row.try_get("git")?,
                    rev: row.try_get("rev")?,
                    title: row.try_get("title")?,
                    summary: row.try_get("summary")?,
                    license: row.try_get("license")?,
                    tags: serde_json::from_value(row.try_get("tags")?)?,
                    installed: false,
                    source_id: None,
                    state: None,
                    active_revision: None,
                    runtime,
                    capabilities: serde_json::from_value(row.try_get("capabilities")?)?,
                })
            })
            .collect()
    }
}
