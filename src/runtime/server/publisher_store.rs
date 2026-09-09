use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;

use super::store::PluginStore;
use crate::runtime::PublishCredentialView;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS plugin_publish_credentials (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    git TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    UNIQUE(tenant_id, git)
);
"#;

pub(super) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(SCHEMA)
        .execute(pool)
        .await
        .context("创建插件发布凭据表失败")?;
    Ok(())
}

impl PluginStore {
    pub async fn create_publish_credential(
        &self,
        tenant_id: &str,
        git: &str,
    ) -> Result<PublishCredentialView> {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let token_hash = token_hash(&token);
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO plugin_publish_credentials (id, tenant_id, git, token_hash) VALUES ($1, $2, $3, $4) ON CONFLICT (tenant_id, git) DO UPDATE SET id = EXCLUDED.id, token_hash = EXCLUDED.token_hash, created_at = now(), revoked_at = NULL",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(git)
        .bind(token_hash)
        .execute(&self.pool)
        .await?;
        Ok(PublishCredentialView {
            id,
            tenant_id: tenant_id.to_owned(),
            git: git.to_owned(),
            token,
        })
    }

    pub async fn publisher_tenant(&self, git: &str, token: &str) -> Result<Option<String>> {
        sqlx::query_scalar(
            "SELECT tenant_id FROM plugin_publish_credentials WHERE git = $1 AND token_hash = $2 AND revoked_at IS NULL",
        )
        .bind(git)
        .bind(token_hash(token))
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn revoke_publish_credential(
        &self,
        tenant_id: &str,
        credential_id: &str,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE plugin_publish_credentials SET revoked_at = now() WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
        )
        .bind(credential_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;
        ensure!(result.rows_affected() == 1, "发布凭证不存在或已撤销");
        Ok(())
    }
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
