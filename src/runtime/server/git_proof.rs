use std::{collections::HashMap, fmt::Write as _};

use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::RepositoryManifest;
use base64::Engine as _;

use crate::runtime::{GitProof, GitTreeProof, PublishPluginRequest};

const MAX_COMMIT_BYTES: usize = 128 * 1024;
const MAX_TREE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TREES: usize = 64;

pub(super) fn verify(
    request: &PublishPluginRequest,
    manifest: &RepositoryManifest,
    artifact: &[u8],
) -> Result<()> {
    let GitProof {
        commit_base64,
        trees,
    } = &request.git_proof;
    let commit = decode(commit_base64, "Git commit")?;
    ensure!(commit.len() <= MAX_COMMIT_BYTES, "Git commit 证明超过配额");
    ensure!(
        object_id("commit", &commit).eq_ignore_ascii_case(&request.rev),
        "Git commit 证明与发布 revision 不一致"
    );
    let root_tree = commit_tree(&commit)?;
    let trees = verified_trees(trees)?;
    verify_blob_path(
        &trees,
        &root_tree,
        "aio-plugin.toml",
        request.manifest_toml.as_bytes(),
    )?;
    let artifact_path = &manifest
        .plugin
        .runtime
        .as_ref()
        .context("发布插件缺少 plugin.runtime")?
        .artifact;
    verify_blob_path(&trees, &root_tree, artifact_path, artifact)
}

fn verified_trees(proofs: &[GitTreeProof]) -> Result<HashMap<String, Vec<u8>>> {
    ensure!(!proofs.is_empty(), "Git 提交证明缺少 tree 对象");
    ensure!(proofs.len() <= MAX_TREES, "Git tree 证明数量超过配额");
    let mut total = 0_usize;
    let mut trees = HashMap::with_capacity(proofs.len());
    for proof in proofs {
        ensure!(is_oid(&proof.oid), "Git tree oid 无效");
        let content = decode(&proof.content_base64, "Git tree")?;
        total = total
            .checked_add(content.len())
            .context("Git tree 大小溢出")?;
        ensure!(total <= MAX_TREE_BYTES, "Git tree 证明超过配额");
        ensure!(
            object_id("tree", &content).eq_ignore_ascii_case(&proof.oid),
            "Git tree 内容与 oid 不一致"
        );
        ensure!(
            trees
                .insert(proof.oid.to_ascii_lowercase(), content)
                .is_none(),
            "Git tree 证明包含重复 oid"
        );
    }
    Ok(trees)
}

fn commit_tree(commit: &[u8]) -> Result<String> {
    let mut tree = None;
    for line in commit.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            break;
        }
        let Some(value) = line.strip_prefix(b"tree ") else {
            continue;
        };
        let value = std::str::from_utf8(value).context("Git commit tree 不是 ASCII")?;
        ensure!(tree.is_none() && is_oid(value), "Git commit tree 头无效");
        tree = Some(value.to_ascii_lowercase());
    }
    tree.context("Git commit 缺少 tree 头")
}

fn verify_blob_path(
    trees: &HashMap<String, Vec<u8>>,
    root_tree: &str,
    path: &str,
    expected: &[u8],
) -> Result<()> {
    let components = path.split('/').collect::<Vec<_>>();
    ensure!(
        !components.is_empty() && components.iter().all(|component| !component.is_empty()),
        "Git 证明路径无效"
    );
    let mut current = root_tree.to_owned();
    for (index, component) in components.iter().enumerate() {
        let tree = trees
            .get(&current)
            .with_context(|| format!("Git 证明缺少 tree: {current}"))?;
        let (mode, oid) = tree_entry(tree, component.as_bytes())?
            .with_context(|| format!("Git commit 不包含路径: {path}"))?;
        if index + 1 == components.len() {
            ensure!(
                matches!(mode, b"100644" | b"100755"),
                "Git commit 路径不是普通文件: {path}"
            );
            ensure!(
                object_id("blob", expected) == oid,
                "上传字节与 Git commit 路径不一致: {path}"
            );
        } else {
            ensure!(mode == b"40000", "Git commit 路径中间节点不是目录: {path}");
            current = oid;
        }
    }
    Ok(())
}

fn tree_entry<'a>(content: &'a [u8], expected_name: &[u8]) -> Result<Option<(&'a [u8], String)>> {
    let mut cursor = 0_usize;
    while cursor < content.len() {
        let mode_end = content[cursor..]
            .iter()
            .position(|byte| *byte == b' ')
            .map(|offset| cursor + offset)
            .context("Git tree 条目缺少 mode 分隔符")?;
        let name_start = mode_end + 1;
        let name_end = content[name_start..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| name_start + offset)
            .context("Git tree 条目缺少名称终止符")?;
        let oid_start = name_end + 1;
        let oid_end = oid_start.checked_add(20).context("Git tree oid 偏移溢出")?;
        ensure!(oid_end <= content.len(), "Git tree 条目的 oid 不完整");
        if &content[name_start..name_end] == expected_name {
            return Ok(Some((
                &content[cursor..mode_end],
                hex_oid(&content[oid_start..oid_end]),
            )));
        }
        cursor = oid_end;
    }
    Ok(None)
}

pub(super) fn object_id(kind: &str, content: &[u8]) -> String {
    let header = format!("{kind} {}\0", content.len());
    let mut hash = sha1_smol::Sha1::new();
    hash.update(header.as_bytes());
    hash.update(content);
    hash.digest().to_string()
}

fn hex_oid(bytes: &[u8]) -> String {
    let mut oid = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut oid, "{byte:02x}").expect("写入 String 不会失败");
    }
    oid
}

fn decode(value: &str, label: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .with_context(|| format!("{label} 证明不是有效 Base64"))
}

fn is_oid(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
