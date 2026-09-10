use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::{
    artifact_path, read_manifest, validate_declared_pages, validate_page_definitions,
};
use az_plugin_package::PluginPackage;
use sha2::{Digest as _, Sha256};

use super::{
    publication_validation::validate_publish_payload,
    repository::{DiscoveredPlugin, RepositoryInstaller, finish_with_cleanup, published_source_id},
};
use crate::runtime::{PageDefinition, PluginRuntime};

impl RepositoryInstaller {
    #[cfg(test)]
    pub async fn publish(&self, package: &PluginPackage) -> Result<DiscoveredPlugin> {
        let staged = self.stage_publish(package).await?;
        self.validate_published(&staged.git, &staged.revision).await
    }

    pub async fn stage_publish(&self, package: &PluginPackage) -> Result<DiscoveredPlugin> {
        let publication = validate_publish_payload(package)?;
        let runtime = publication
            .manifest
            .plugin
            .runtime
            .as_ref()
            .context("插件包缺少运行目标")?;
        let pages = if runtime.kind == PluginRuntime::PageDefinition {
            let pages: Vec<PageDefinition> = serde_json::from_slice(&publication.artifact)
                .context("解析插件包 PageDefinition 失败")?;
            validate_page_definitions(&pages)?;
            validate_declared_pages(&publication.manifest, &pages)?;
            pages
        } else {
            Vec::new()
        };
        let _cache_guard = self.cache_operations.lock().await;
        tokio::fs::create_dir_all(&self.cache_root)
            .await
            .context("创建插件缓存目录失败")?;
        let staging = self
            .cache_root
            .join(format!("staging-package-{}", uuid::Uuid::new_v4()));
        let result = async {
            let directory = self.cache_root.join(&package.rev);
            if directory.exists() {
                ensure!(
                    read_manifest(&directory)? == publication.manifest,
                    "插件包缓存清单与内容版本不一致"
                );
                let artifact = artifact_path(&directory, &runtime.artifact)?;
                ensure!(
                    format!("{:x}", Sha256::digest(tokio::fs::read(artifact).await?))
                        == package.artifact_sha256,
                    "插件包缓存 artifact SHA-256 不一致"
                );
            } else {
                tokio::fs::create_dir_all(&staging).await?;
                tokio::fs::write(staging.join("aio-plugin.toml"), &package.manifest_toml).await?;
                let artifact = staging.join(&runtime.artifact);
                tokio::fs::create_dir_all(artifact.parent().context("插件 artifact 缺少父目录")?)
                    .await?;
                tokio::fs::write(&artifact, &publication.artifact).await?;
                self.ensure_cache_quota(true).await?;
                tokio::fs::rename(&staging, &directory)
                    .await
                    .context("原子发布插件包缓存失败")?;
            }
            Ok(DiscoveredPlugin {
                source_id: published_source_id(&package.git),
                git: package.git.clone(),
                revision: package.rev.clone(),
                runtime: runtime.kind,
                manifest: serde_json::to_value(&publication.manifest.plugin)?,
                pages,
                artifact: runtime.artifact.clone(),
            })
        }
        .await;
        finish_with_cleanup(result, &staging).await
    }
}
