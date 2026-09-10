use std::env;

use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::{parse_manifest, read_manifest, validate_host_compatibility};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use super::{
    git_proof,
    remote_access::validate_git,
    repository::{RepositoryInstaller, is_full_revision},
};
use crate::runtime::{MarketplaceEntry, PluginRuntime, PublishPluginRequest};

const MAX_PUBLISHED_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;
const MAX_PUBLISHED_MANIFEST_BYTES: usize = 128 * 1024;

pub(super) struct ValidatedPublication {
    pub manifest: az_plugin_manifest::RepositoryManifest,
    pub artifact: Vec<u8>,
}

pub(super) fn validate_publish_payload(
    request: &PublishPluginRequest,
) -> Result<ValidatedPublication> {
    validate_git(&request.git)?;
    ensure!(
        is_full_revision(&request.rev),
        "发布插件必须使用完整提交 SHA"
    );
    ensure!(
        request.manifest_toml.len() <= MAX_PUBLISHED_MANIFEST_BYTES,
        "发布插件清单不能超过 {MAX_PUBLISHED_MANIFEST_BYTES} 字节"
    );
    ensure!(
        request.artifact_sha256.len() == 64
            && request
                .artifact_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "artifact_sha256 必须是完整 SHA-256"
    );
    let artifact = base64::engine::general_purpose::STANDARD
        .decode(&request.artifact_base64)
        .context("发布 artifact 不是有效 Base64")?;
    ensure!(
        artifact.len() <= MAX_PUBLISHED_ARTIFACT_BYTES,
        "发布 artifact 不能超过 {MAX_PUBLISHED_ARTIFACT_BYTES} 字节"
    );
    let actual_digest = format!("{:x}", Sha256::digest(&artifact));
    ensure!(
        actual_digest.eq_ignore_ascii_case(&request.artifact_sha256),
        "发布 artifact SHA-256 不匹配"
    );
    let manifest = parse_manifest(&request.manifest_toml)?;
    validate_host_compatibility(&manifest, env!("CARGO_PKG_VERSION"))?;
    ensure!(
        manifest.plugin.marketplace.is_some(),
        "在线发布插件必须声明 [plugin.marketplace]"
    );
    let runtime = manifest
        .plugin
        .runtime
        .as_ref()
        .context("发布插件缺少 plugin.runtime")?;
    ensure_publish_runtime(runtime.kind)?;
    ensure_publish_capabilities(&manifest)?;
    git_proof::verify(request, &manifest, &artifact)?;
    Ok(ValidatedPublication { manifest, artifact })
}

pub(super) fn ensure_publish_runtime(runtime: PluginRuntime) -> Result<()> {
    ensure!(
        matches!(
            runtime,
            PluginRuntime::PageDefinition | PluginRuntime::WasmComponent | PluginRuntime::Process
        ),
        "发布接口只接受 page-definition、wasm-component 或 process artifact"
    );
    Ok(())
}

pub(super) fn ensure_publish_capabilities(
    manifest: &az_plugin_manifest::RepositoryManifest,
) -> Result<()> {
    let capabilities = &manifest.plugin.capabilities;
    ensure!(
        capabilities.network.is_empty()
            && capabilities.filesystem.is_empty()
            && !capabilities.database,
        "当前在线发布宿主未授予网络、文件系统或数据库能力"
    );
    Ok(())
}

impl RepositoryInstaller {
    pub fn published_marketplace_entry(
        &self,
        git: &str,
        revision: &str,
    ) -> Result<MarketplaceEntry> {
        validate_git(git)?;
        ensure!(is_full_revision(revision), "发布插件必须使用完整提交 SHA");
        let manifest = read_manifest(&self.cache_root.join(revision))?;
        let metadata = manifest
            .plugin
            .marketplace
            .as_ref()
            .context("在线发布插件必须声明 [plugin.marketplace]")?;
        Ok(MarketplaceEntry {
            git: git.to_owned(),
            rev: revision.to_owned(),
            title: metadata.title.clone(),
            summary: metadata.summary.clone(),
            license: metadata.license.clone(),
            tags: metadata.tags.clone(),
            installed: false,
            source_id: None,
            state: None,
            active_revision: None,
            runtime: manifest.plugin.runtime.map(|runtime| runtime.kind),
            capabilities: manifest.plugin.capabilities,
        })
    }
}
