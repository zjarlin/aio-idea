use std::{
    env,
    io::Cursor,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context as _, Result, ensure};
use flate2::read::GzDecoder;
use serde_json::Value;
use tar::Archive;
use tokio::process::Command;

use crate::runtime::{MarketplaceEntry, PageDefinition, PluginRuntime};

use super::manifest;

const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;

pub struct DiscoveredPlugin {
    pub source_id: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub manifest: Value,
    pub pages: Vec<PageDefinition>,
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
            let manifest_path = checkout_root.join("aio-plugin.toml");
            let manifest_text = tokio::fs::read_to_string(&manifest_path)
                .await
                .with_context(|| format!("读取插件清单失败: {}", manifest_path.display()))?;
            let manifest = manifest::parse(&manifest_text)?;
            let artifact = safe_artifact(&checkout_root, &manifest.plugin.runtime.artifact)?;
            let pages = match manifest.plugin.runtime.kind {
                PluginRuntime::PageDefinition => serde_json::from_slice::<Vec<PageDefinition>>(
                    &tokio::fs::read(&artifact).await.with_context(|| {
                        format!("读取 PageDefinition 失败: {}", artifact.display())
                    })?,
                )
                .context("解析 PageDefinition 失败")?,
                PluginRuntime::WasmComponent => {
                    let artifact = artifact.clone();
                    tokio::task::spawn_blocking(move || super::wasm::load_pages(&artifact))
                        .await
                        .context("等待 Wasm Component 验证失败")??
                }
                PluginRuntime::Process => anyhow::bail!("process 插件必须由进程监督器激活"),
                PluginRuntime::RustSource => {
                    anyhow::bail!("rust-source 插件必须通过整套发布切换")
                }
            };
            validate_pages(&pages)?;
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
                runtime: manifest.plugin.runtime.kind,
                manifest: serde_json::to_value(&manifest.plugin)?,
                pages,
            })
        }
        .await;
        finish_with_cleanup(result, &staging).await
    }

    pub async fn registry(&self, source: &str) -> Result<Vec<MarketplaceEntry>> {
        if !source.ends_with(".git") {
            ensure!(source.starts_with("https://"), "市场索引必须使用 HTTPS");
            return reqwest::get(source)
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
        safe_artifact(&self.cache_root.join(revision), relative)
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

fn validate_git(git: &str) -> Result<()> {
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

fn safe_artifact(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    ensure!(
        !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "插件 artifact 必须是仓库内相对路径"
    );
    let artifact = root.join(path);
    ensure!(artifact.is_file(), "插件 artifact 不存在: {relative}");
    Ok(artifact)
}

fn validate_pages(pages: &[PageDefinition]) -> Result<()> {
    ensure!(!pages.is_empty(), "插件至少需要贡献一个页面");
    let mut ids = std::collections::HashSet::new();
    for page in pages {
        ensure!(
            !page.id.trim().is_empty() && !page.label.trim().is_empty(),
            "插件页面 id 和标题不能为空"
        );
        ensure!(
            !page.scene.id.trim().is_empty() && !page.scene.label.trim().is_empty(),
            "插件页面场景不能为空"
        );
        ensure!(
            ids.insert(page.id.as_str()),
            "插件页面 id 重复: {}",
            page.id
        );
    }
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
}
