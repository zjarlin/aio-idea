#!/usr/bin/env zsh

set -euo pipefail

readonly deploy_host="${AIO_DEPLOY_HOST:-root@192.168.31.252}"
readonly deploy_root="${AIO_DEPLOY_ROOT:-/opt/aio-public-shell}"
readonly target="x86_64-unknown-linux-gnu.2.17"
readonly target_directory="${target%%.*}"
readonly repository="$(git rev-parse --show-toplevel)"
readonly revision="${1:-$(git -C "$repository" rev-parse HEAD)}"

if (( $# > 1 )); then
    print -u2 "用法: $0 [完整 Git SHA]"
    exit 64
fi
if [[ ! "$revision" =~ '^[0-9a-f]{40}$' ]]; then
    print -u2 "发布版本必须是完整的 40 位 Git SHA"
    exit 64
fi
if [[ -n "$(git -C "$repository" status --porcelain)" ]]; then
    print -u2 "发布仓库存在未提交修改，请先提交后再发布"
    exit 1
fi

readonly workspace="$(mktemp -d "${TMPDIR:-/tmp}/aio-release-worktree.XXXXXX")"
readonly artifact="$(mktemp -d "${TMPDIR:-/tmp}/aio-release-artifact.XXXXXX")"
cleanup() {
    git -C "$repository" worktree remove --force "$workspace/source" 2>/dev/null || true
    rm -rf "$workspace" "$artifact"
}
trap cleanup EXIT

git -C "$repository" worktree add --detach "$workspace/source" "$revision"
git -C "$workspace/source" submodule update --init --recursive

cd "$workspace/source"
export CARGO_TARGET_DIR="$workspace/source/target"

print "验证服务端"
cargo test --no-default-features --features server
print "检查 Web 客户端"
cargo check --no-default-features --features web
print "构建 glibc 2.17 服务端"
cargo zigbuild --release --target "$target" --no-default-features --features server
print "构建 Web 资产"
dx build --platform web --release

readonly release="$artifact/release"
mkdir -p "$release"
cp "$CARGO_TARGET_DIR/$target_directory/release/aio-public-shell" "$release/aio-public-shell"
cp -R target/dx/aio-public-shell/release/web "$release/web"
cp aio.toml "$release/aio.toml"

readonly incoming="$deploy_root/releases/.incoming-$revision"
readonly remote_release="$deploy_root/releases/$revision"

ssh "$deploy_host" "set -eu
test ! -e '$remote_release'
rm -rf '$incoming'
mkdir -p '$incoming'"
print "上传候选发布物"
tar -C "$release" -cf - . | ssh "$deploy_host" "tar -C '$incoming' -xf -"

print "切换 252 发布物"
ssh "$deploy_host" "set -eu
deploy_root='$deploy_root'
incoming='$incoming'
remote_release='$remote_release'
previous=\$(readlink -f \"\$deploy_root/current\")
test -x \"\$incoming/aio-public-shell\"
test -f \"\$incoming/aio.toml\"
test -f \"\$incoming/web/index.html\"
chown -R root:aio-shell \"\$incoming\"
chmod -R u=rwX,g=rX,o= \"\$incoming\"
mv \"\$incoming\" \"\$remote_release\"
ln -s \"\$remote_release\" \"\$deploy_root/.next\"
mv -Tf \"\$deploy_root/.next\" \"\$deploy_root/current\"

if systemctl restart aio-plugin-supervisor.service \\
    && systemctl restart aio-public-shell.service \\
    && curl --fail --silent --show-error --retry 12 --retry-delay 1 http://127.0.0.1:3080/health >/dev/null; then
    printf '已激活 %s\\n' '$revision'
else
    ln -s \"\$previous\" \"\$deploy_root/.rollback\"
    mv -Tf \"\$deploy_root/.rollback\" \"\$deploy_root/current\"
    systemctl restart aio-plugin-supervisor.service
    systemctl restart aio-public-shell.service
    curl --fail --silent --show-error --retry 12 --retry-delay 1 http://127.0.0.1:3080/health >/dev/null
    printf '发布失败，已恢复 %s\\n' \"\$previous\" >&2
    exit 1
fi"
