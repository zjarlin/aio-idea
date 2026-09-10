use std::{
    env,
    io::Cursor,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::{
    artifact_path, parse_manifest, read_manifest, validate_declared_pages,
    validate_host_compatibility, validate_page_definitions, validate_repository,
};
use base64::Engine as _;
use flate2::read::GzDecoder;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tar::Archive;
use tokio::process::Command;

use crate::runtime::{MarketplaceEntry, PageDefinition, PluginRuntime, PublishPluginRequest};

const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PUBLISHED_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;
const MAX_PUBLISHED_MANIFEST_BYTES: usize = 128 * 1024;
const REGISTRY_TIMEOUT: Duration = Duration::from_secs(8);

pub struct DiscoveredPlugin {
    pub source_id: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub manifest: Value,
    pub pages: Vec<PageDefinition>,
    pub artifact: String,
}

pub struct RepositoryInstaller {
    cache_root: PathBuf,
}

impl RepositoryInstaller {
    pub fn new(cache_root: PathBuf) -> Self {
        Self { cache_root }
    }

    pub async fn discover(&self, git: &str, revision: Option<&str>) -> Result<DiscoveredPlugin> {
        validate_git(git)?;
        tokio::fs::create_dir_all(&self.cache_root)
            .await
            .context("创建插件缓存目录失败")?;
        let staging = self
            .cache_root
            .join(format!("staging-{}", uuid::Uuid::new_v4()));
        let result = async {
            let archive = github_archive(git, revision)?;
            let (checkout_root, resolved) = if let Some((archive, resolved)) = archive {
                (download_archive(archive, &staging).await?, resolved)
            } else {
                run_git(
                    None,
                    &[
                        "clone",
                        "--quiet",
                        "--no-checkout",
                        git,
                        staging.to_str().context("插件缓存路径不是 Unicode")?,
                    ],
                )
                .await?;
                let checkout = revision.unwrap_or("HEAD");
                run_git(Some(&staging), &["checkout", "--quiet", checkout]).await?;
                let resolved = git_output(&staging, &["rev-parse", "HEAD"]).await?;
                (staging.clone(), resolved)
            };
            ensure!(
                resolved.len() == 40 && resolved.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Git 未解析出完整提交 SHA"
            );
            let manifest = read_manifest(&checkout_root)?;
            validate_host_compatibility(&manifest, env!("CARGO_PKG_VERSION"))?;
            let validation_root = checkout_root.clone();
            let report = tokio::task::spawn_blocking(move || validate_repository(&validation_root))
                .await
                .context("等待插件 artifact 校验失败")??;
            let runtime = manifest
                .plugin
                .runtime
                .as_ref()
                .context("公网宿主只接受声明 plugin.runtime 的插件")?;
            let runtime_kind = runtime.kind;
            if runtime_kind == PluginRuntime::WasmComponent {
                ensure!(
                    manifest.plugin.capabilities.network.is_empty()
                        && manifest.plugin.capabilities.filesystem.is_empty()
                        && !manifest.plugin.capabilities.database,
                    "当前 Wasm Component 宿主未授予网络、文件系统或数据库能力"
                );
            }
            let artifact = report.artifact.context("运行时插件缺少 artifact")?;
            let pages = match runtime_kind {
                PluginRuntime::PageDefinition => serde_json::from_slice::<Vec<PageDefinition>>(
                    &tokio::fs::read(&artifact).await.with_context(|| {
                        format!("读取 PageDefinition 失败: {}", artifact.display())
                    })?,
                )
                .context("解析 PageDefinition 失败")?,
                PluginRuntime::WasmComponent => Vec::new(),
                PluginRuntime::Process => Vec::new(),
                PluginRuntime::RustSource => {
                    anyhow::bail!("rust-source 插件必须通过整套发布切换")
                }
            };
            if runtime_kind == PluginRuntime::PageDefinition {
                validate_page_definitions(&pages)?;
                validate_declared_pages(&manifest, &pages)?;
            }
            let final_directory = self.cache_root.join(&resolved);
            if final_directory.exists() {
                tokio::fs::remove_dir_all(&staging).await?;
            } else {
                tokio::fs::rename(&checkout_root, &final_directory)
                    .await
                    .context("原子发布插件缓存失败")?;
            }
            Ok(DiscoveredPlugin {
                source_id: uuid::Uuid::new_v4().to_string(),
                git: git.to_owned(),
                revision: resolved,
                runtime: runtime_kind,
                manifest: serde_json::to_value(&manifest.plugin)?,
                pages,
                artifact: runtime.artifact.clone(),
            })
        }
        .await;
        finish_with_cleanup(result, &staging).await
    }

    #[cfg(test)]
    pub async fn publish(&self, request: &PublishPluginRequest) -> Result<DiscoveredPlugin> {
        let staged = self.stage_publish(request).await?;
        self.validate_published(&staged.git, &staged.revision).await
    }

    pub async fn stage_publish(&self, request: &PublishPluginRequest) -> Result<DiscoveredPlugin> {
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
        let artifact_bytes = base64::engine::general_purpose::STANDARD
            .decode(&request.artifact_base64)
            .context("发布 artifact 不是有效 Base64")?;
        ensure!(
            artifact_bytes.len() <= MAX_PUBLISHED_ARTIFACT_BYTES,
            "发布 artifact 不能超过 {MAX_PUBLISHED_ARTIFACT_BYTES} 字节"
        );
        let actual_digest = format!("{:x}", Sha256::digest(&artifact_bytes));
        ensure!(
            actual_digest.eq_ignore_ascii_case(&request.artifact_sha256),
            "发布 artifact SHA-256 不匹配"
        );
        let manifest = parse_manifest(&request.manifest_toml)?;
        validate_host_compatibility(&manifest, env!("CARGO_PKG_VERSION"))?;
        let runtime = manifest
            .plugin
            .runtime
            .as_ref()
            .context("发布插件缺少 plugin.runtime")?;
        ensure_publish_runtime(runtime.kind)?;
        ensure_publish_capabilities(&manifest)?;
        tokio::fs::create_dir_all(&self.cache_root)
            .await
            .context("创建插件缓存目录失败")?;
        let staging = self
            .cache_root
            .join(format!("staging-publish-{}", uuid::Uuid::new_v4()));
        let result = async {
            tokio::fs::create_dir_all(&staging).await?;
            tokio::fs::write(staging.join("aio-plugin.toml"), &request.manifest_toml).await?;
            let artifact_target = staging.join(&runtime.artifact);
            let parent = artifact_target
                .parent()
                .context("发布 artifact 缺少父目录")?;
            tokio::fs::create_dir_all(parent).await?;
            tokio::fs::write(&artifact_target, &artifact_bytes).await?;
            let final_directory = self.cache_root.join(&request.rev);
            if final_directory.exists() {
                let existing_manifest = read_manifest(&final_directory)?;
                ensure!(
                    existing_manifest == manifest,
                    "同一提交 SHA 已存在不同的发布清单"
                );
                let existing_artifact = artifact_path(&final_directory, &runtime.artifact)?;
                let existing_digest =
                    format!("{:x}", Sha256::digest(std::fs::read(existing_artifact)?));
                ensure!(
                    existing_digest.eq_ignore_ascii_case(&request.artifact_sha256),
                    "同一提交 SHA 已存在不同的发布 artifact"
                );
            } else {
                tokio::fs::rename(&staging, &final_directory)
                    .await
                    .context("原子发布 artifact 缓存失败")?;
            }
            let pages = if runtime.kind == PluginRuntime::PageDefinition {
                serde_json::from_slice::<Vec<PageDefinition>>(
                    &tokio::fs::read(artifact_path(&final_directory, &runtime.artifact)?).await?,
                )
                .context("解析已发布 PageDefinition 失败")?
            } else {
                Vec::new()
            };
            if runtime.kind == PluginRuntime::PageDefinition {
                validate_page_definitions(&pages)?;
                validate_declared_pages(&manifest, &pages)?;
            }
            Ok(DiscoveredPlugin {
                source_id: published_source_id(&request.git),
                git: request.git.clone(),
                revision: request.rev.clone(),
                runtime: runtime.kind,
                manifest: serde_json::to_value(&manifest.plugin)?,
                pages,
                artifact: runtime.artifact.clone(),
            })
        }
        .await;
        finish_with_cleanup(result, &staging).await
    }

    pub async fn validate_published(&self, git: &str, revision: &str) -> Result<DiscoveredPlugin> {
        validate_git(git)?;
        ensure!(is_full_revision(revision), "发布插件必须使用完整提交 SHA");
        let root = self.cache_root.join(revision);
        let manifest = read_manifest(&root)?;
        validate_host_compatibility(&manifest, env!("CARGO_PKG_VERSION"))?;
        let runtime = manifest
            .plugin
            .runtime
            .as_ref()
            .context("发布插件缺少 plugin.runtime")?;
        ensure_publish_runtime(runtime.kind)?;
        ensure_publish_capabilities(&manifest)?;
        let validation_root = root.clone();
        let report = tokio::task::spawn_blocking(move || validate_repository(&validation_root))
            .await
            .context("等待发布 artifact 校验失败")??;
        let pages = if runtime.kind == PluginRuntime::PageDefinition {
            serde_json::from_slice::<Vec<PageDefinition>>(
                &tokio::fs::read(artifact_path(&root, &runtime.artifact)?).await?,
            )
            .context("解析已发布 PageDefinition 失败")?
        } else {
            Vec::new()
        };
        Ok(DiscoveredPlugin {
            source_id: published_source_id(git),
            git: git.to_owned(),
            revision: revision.to_owned(),
            runtime: report.runtime,
            manifest: serde_json::to_value(&manifest.plugin)?,
            pages,
            artifact: runtime.artifact.clone(),
        })
    }

    pub async fn registry(&self, source: &str) -> Result<Vec<MarketplaceEntry>> {
        if !source.ends_with(".git") {
            ensure!(source.starts_with("https://"), "市场索引必须使用 HTTPS");
            let client = reqwest::Client::builder()
                .connect_timeout(REGISTRY_TIMEOUT)
                .timeout(REGISTRY_TIMEOUT)
                .build()
                .context("创建市场索引 HTTP 客户端失败")?;
            return client
                .get(source)
                .send()
                .await
                .context("请求市场索引失败")?
                .error_for_status()
                .context("市场索引返回失败状态")?
                .json()
                .await
                .context("解析市场索引失败");
        }
        validate_git(source)?;
        tokio::fs::create_dir_all(&self.cache_root).await?;
        let checkout = self
            .cache_root
            .join(format!("registry-{}", uuid::Uuid::new_v4()));
        let result = async {
            run_git(
                None,
                &[
                    "clone",
                    "--quiet",
                    "--depth",
                    "1",
                    source,
                    checkout.to_str().context("市场缓存路径不是 Unicode")?,
                ],
            )
            .await?;
            let primary = checkout.join("marketplace/index.json");
            let path = if primary.is_file() {
                primary
            } else {
                checkout.join("index.json")
            };
            serde_json::from_slice(
                &tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("读取 Git 市场索引失败: {}", path.display()))?,
            )
            .context("解析 Git 市场索引失败")
        }
        .await;
        finish_with_cleanup(result, &checkout).await
    }

    pub fn artifact(&self, revision: &str, relative: &str) -> Result<PathBuf> {
        ensure!(
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "插件 revision 必须是完整提交 SHA"
        );
        artifact_path(&self.cache_root.join(revision), relative)
    }

    pub fn validate_pages(&self, revision: &str, pages: &[PageDefinition]) -> Result<()> {
        ensure!(
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "插件 revision 必须是完整提交 SHA"
        );
        let manifest = read_manifest(&self.cache_root.join(revision))?;
        validate_page_definitions(pages)?;
        validate_declared_pages(&manifest, pages)
    }
}

async fn finish_with_cleanup<T>(result: Result<T>, directory: &Path) -> Result<T> {
    let cleanup = async {
        if tokio::fs::try_exists(directory).await? {
            tokio::fs::remove_dir_all(directory).await?;
        }
        Ok::<_, std::io::Error>(())
    }
    .await;
    match result {
        Ok(value) => {
            cleanup.context("清理插件临时目录失败")?;
            Ok(value)
        }
        Err(error) => {
            let _ = cleanup;
            Err(error)
        }
    }
}

fn github_archive(git: &str, revision: Option<&str>) -> Result<Option<(reqwest::Url, String)>> {
    let Some(revision) = revision.filter(|value| is_full_revision(value)) else {
        return Ok(None);
    };
    let source = reqwest::Url::parse(git).context("插件 Git 地址无效")?;
    if source.host_str() != Some("github.com") {
        return Ok(None);
    }
    let segments = source
        .path_segments()
        .context("GitHub 仓库地址缺少路径")?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() != 2 {
        return Ok(None);
    }
    let repository = segments[1]
        .strip_suffix(".git")
        .context("GitHub 仓库地址必须以 .git 结尾")?;
    ensure!(
        [segments[0], repository].into_iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        }),
        "GitHub 仓库路径包含无效字符"
    );
    let archive = reqwest::Url::parse(&format!(
        "https://codeload.github.com/{}/{repository}/tar.gz/{revision}",
        segments[0]
    ))
    .context("构造 GitHub 提交归档地址失败")?;
    Ok(Some((archive, revision.to_owned())))
}

fn is_full_revision(revision: &str) -> bool {
    revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn published_source_id(git: &str) -> String {
    format!("publish-{:x}", Sha256::digest(git.as_bytes()))
}

fn ensure_publish_runtime(runtime: PluginRuntime) -> Result<()> {
    ensure!(
        matches!(
            runtime,
            PluginRuntime::PageDefinition | PluginRuntime::WasmComponent | PluginRuntime::Process
        ),
        "发布接口只接受 page-definition、wasm-component 或 process artifact"
    );
    Ok(())
}

fn ensure_publish_capabilities(manifest: &az_plugin_manifest::RepositoryManifest) -> Result<()> {
    let capabilities = &manifest.plugin.capabilities;
    ensure!(
        capabilities.network.is_empty()
            && capabilities.filesystem.is_empty()
            && !capabilities.database,
        "当前在线发布宿主未授予网络、文件系统或数据库能力"
    );
    Ok(())
}

async fn download_archive(source: reqwest::Url, staging: &Path) -> Result<PathBuf> {
    let response = reqwest::get(source)
        .await
        .context("下载 GitHub 提交归档失败")?
        .error_for_status()
        .context("GitHub 提交归档返回失败状态")?;
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_ARCHIVE_BYTES),
        "GitHub 提交归档超过 32 MiB 配额"
    );
    let content = response.bytes().await.context("读取 GitHub 提交归档失败")?;
    ensure!(
        content.len() as u64 <= MAX_ARCHIVE_BYTES,
        "GitHub 提交归档超过 32 MiB 配额"
    );
    let staging = staging.to_owned();
    let extraction = staging.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let decoder = GzDecoder::new(Cursor::new(content));
        Archive::new(decoder)
            .unpack(&extraction)
            .context("解压 GitHub 提交归档失败")
    })
    .await
    .context("等待 GitHub 提交归档解压失败")??;
    let mut entries = tokio::fs::read_dir(&staging)
        .await
        .context("读取 GitHub 提交归档目录失败")?;
    let root = entries.next_entry().await?.context("GitHub 提交归档为空")?;
    ensure!(
        root.file_type().await?.is_dir(),
        "GitHub 提交归档根不是目录"
    );
    ensure!(
        entries.next_entry().await?.is_none(),
        "GitHub 提交归档包含多个根目录"
    );
    Ok(root.path())
}

pub(super) fn validate_git(git: &str) -> Result<()> {
    ensure!(
        git.starts_with("https://") && git.ends_with(".git"),
        "插件来源必须是 HTTPS Git 仓库"
    );
    ensure!(
        !git.contains(char::is_whitespace),
        "插件 Git 地址不能包含空白字符"
    );
    Ok(())
}

async fn run_git(directory: Option<&Path>, arguments: &[&str]) -> Result<()> {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .kill_on_drop(true)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let output = tokio::time::timeout(git_timeout(), command.output())
        .await
        .context("执行 Git 超时")??;
    ensure!(
        output.status.success(),
        "Git 失败: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

async fn git_output(directory: &Path, arguments: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .current_dir(directory)
        .kill_on_drop(true);
    let output = tokio::time::timeout(git_timeout(), command.output())
        .await
        .context("执行 Git 超时")??;
    ensure!(
        output.status.success(),
        "Git 失败: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn git_timeout() -> Duration {
    env::var("AIO_GIT_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_github_archive_for_full_revision() -> Result<()> {
        let revision = "0123456789012345678901234567890123456789";
        let (archive, resolved) =
            github_archive("https://github.com/example/aio-plugin.git", Some(revision))?
                .context("应生成 GitHub 归档")?;

        assert_eq!(resolved, revision);
        assert_eq!(
            archive.as_str(),
            "https://codeload.github.com/example/aio-plugin/tar.gz/0123456789012345678901234567890123456789"
        );
        Ok(())
    }

    #[test]
    fn keeps_git_for_non_github_or_symbolic_revision() -> Result<()> {
        assert!(
            github_archive(
                "https://git.example.com/plugin.git",
                Some("0".repeat(40).as_str())
            )?
            .is_none()
        );
        assert!(github_archive("https://github.com/example/plugin.git", Some("main"))?.is_none());
        Ok(())
    }

    #[test]
    fn rejects_capabilities_not_granted_to_online_publications() -> Result<()> {
        let manifest = parse_manifest(
            "[plugin.runtime]\nkind = 'process'\nartifact = 'plugin.js'\ncontainer_image = 'node:22@sha256:6c74791e557ce11fc957704f6d4fe134a7bc8d6f5ca4403205b2966bd488f6b3'\nentrypoint = ['node', '{artifact}']\nhealth_check = '/health'\n\n[plugin.capabilities]\ndatabase = true\n",
        )?;

        let error = ensure_publish_capabilities(&manifest)
            .expect_err("未授权的数据库能力必须在暂存前被拒绝");

        assert!(error.to_string().contains("未授予"));
        Ok(())
    }

    #[tokio::test]
    async fn publishes_validated_page_definition_artifact() -> Result<()> {
        let artifact = br#"[{"id":"published-page","label":"Published","icon":"box","scene":{"id":"community","label":"Community"},"required_permission":null,"body":{"kind":"text","title":"Published","content":"from CI"}}]"#;
        let request = PublishPluginRequest {
            git: "https://github.com/example/aio-plugin-published.git".to_owned(),
            rev: "a".repeat(40),
            manifest_toml: r#"
[plugin.runtime]
kind = "page-definition"
artifact = "dist/pages.json"

[plugin.capabilities]
network = []
filesystem = []
database = false

[[plugin.subplugins]]
id = "published"
pages = ["published-page"]
"#
            .to_owned(),
            artifact_base64: base64::engine::general_purpose::STANDARD.encode(artifact),
            artifact_sha256: format!("{:x}", Sha256::digest(artifact)),
            tenant_id: None,
        };
        let temporary = tempfile::tempdir()?;
        let installer = RepositoryInstaller::new(temporary.path().join("cache"));

        let published = installer.publish(&request).await?;

        assert_eq!(published.runtime, PluginRuntime::PageDefinition);
        assert_eq!(published.pages.len(), 1);
        assert!(
            installer
                .artifact(&request.rev, "dist/pages.json")?
                .is_file()
        );
        Ok(())
    }

    #[tokio::test]
    async fn publishes_validated_process_artifact() -> Result<()> {
        let artifact = b"committed process artifact";
        let request = PublishPluginRequest {
            git: "https://github.com/example/aio-plugin-process.git".to_owned(),
            rev: "c".repeat(40),
            manifest_toml: r#"
[plugin.runtime]
kind = "process"
artifact = "dist/plugin.jar"
host_version = ">=2026.9.9"
container_image = "eclipse-temurin:21-jre@sha256:5c67d24ee8e3dd810b2a0cb6c3827ced2ac5d22729538f90b36c2b9d77678bb8"
entrypoint = ["java", "-jar", "{artifact}"]
health_check = "/health"
shutdown_timeout_seconds = 10

[plugin.capabilities]
network = []
filesystem = []
database = false
"#
            .to_owned(),
            artifact_base64: base64::engine::general_purpose::STANDARD.encode(artifact),
            artifact_sha256: format!("{:x}", Sha256::digest(artifact)),
            tenant_id: None,
        };
        let temporary = tempfile::tempdir()?;
        let installer = RepositoryInstaller::new(temporary.path().join("cache"));

        let published = installer.publish(&request).await?;

        assert_eq!(published.runtime, PluginRuntime::Process);
        assert!(published.pages.is_empty());
        assert!(
            installer
                .artifact(&request.rev, "dist/plugin.jar")?
                .is_file()
        );
        Ok(())
    }

    #[tokio::test]
    async fn rejects_published_artifact_with_mismatched_digest() -> Result<()> {
        let request = PublishPluginRequest {
            git: "https://github.com/example/aio-plugin-published.git".to_owned(),
            rev: "b".repeat(40),
            manifest_toml: "[plugin.runtime]\nkind = 'page-definition'\nartifact = 'pages.json'"
                .to_owned(),
            artifact_base64: base64::engine::general_purpose::STANDARD.encode(b"[]"),
            artifact_sha256: "0".repeat(64),
            tenant_id: None,
        };
        let temporary = tempfile::tempdir()?;
        let installer = RepositoryInstaller::new(temporary.path().join("cache"));

        let error = match installer.publish(&request).await {
            Ok(_) => panic!("摘要错误的 artifact 不能发布"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("SHA-256 不匹配"));
        Ok(())
    }
}
