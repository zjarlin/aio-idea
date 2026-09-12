#!/usr/bin/env zsh

set -euo pipefail

export COPYFILE_DISABLE=1

readonly deploy_host="${AIO_DEPLOY_HOST:-root@192.168.31.252}"
readonly deploy_root="${AIO_DEPLOY_ROOT:-/opt/aio-idea}"
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
dx build --platform web --release --debug-symbols false
npm ci --prefix deploy --ignore-scripts --no-audit --no-fund
node deploy/prepare-web.cjs

readonly release="$artifact/release"
mkdir -p "$release"
cp "$CARGO_TARGET_DIR/$target_directory/release/aio-idea" "$release/aio-idea"
cp -R target/dx/aio-idea/release/web/public "$release/web"
cp aio.toml "$release/aio.toml"
mkdir -p "$release/systemd"
cp deploy/aio-plugin-supervisor.service "$release/systemd/aio-plugin-supervisor.service"
cp deploy/252/aio-idea.service "$release/systemd/aio-idea.service"

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
previous=''
if [ -L \"\$deploy_root/current\" ]; then
    previous=\$(readlink -f \"\$deploy_root/current\")
fi
test -x \"\$incoming/aio-idea\"
test -f \"\$incoming/aio.toml\"
test -f \"\$incoming/web/index.html\"
test -f \"\$incoming/systemd/aio-plugin-supervisor.service\"
test -f \"\$incoming/systemd/aio-idea.service\"
install -d -o aio-shell -g aio-shell -m 0750 \"\$deploy_root/file-storage\"
chown -R root:aio-shell \"\$incoming\"
chmod -R u=rwX,g=rX,o= \"\$incoming\"
mv \"\$incoming\" \"\$remote_release\"
backup=\$(mktemp -d \"\$deploy_root/releases/.systemd-backup.XXXXXX\")
for unit in aio-plugin-supervisor.service aio-idea.service; do
    if [ -f \"/etc/systemd/system/\$unit\" ]; then
        cp -p \"/etc/systemd/system/\$unit\" \"\$backup/\$unit\"
    else
        touch \"\$backup/\$unit.missing\"
    fi
done
supervisor_was_enabled=0
shell_was_enabled=0
if systemctl is-enabled aio-plugin-supervisor.service >/dev/null 2>&1; then
    supervisor_was_enabled=1
fi
if systemctl is-enabled aio-idea.service >/dev/null 2>&1; then
    shell_was_enabled=1
fi
rm -f \"\$deploy_root/.next\" \"\$deploy_root/.rollback\"

wait_for_health() {
    url=\$1
    seconds=\$2
    deadline=\$((\$(date +%s) + seconds))
    while [ \$(date +%s) -lt \"\$deadline\" ]; do
        if curl --fail --silent --show-error --connect-timeout 1 --max-time 2 \"\$url\" >/dev/null; then
            return 0
        fi
        sleep 1
    done
    return 1
}

release_binary_matches_main_pid() {
    expected_binary=\$1
    expected_path=\$(readlink -f "\$expected_binary" 2>/dev/null) || return 1
    current_path=\$(readlink -f "\$deploy_root/current/aio-idea" 2>/dev/null) || return 1
    [ "\$current_path" = "\$expected_path" ] || return 1
    systemctl is-active --quiet aio-idea.service || return 1
    main_pid_line=\$(systemctl show aio-idea.service --property MainPID 2>/dev/null) || return 1
    main_pid=\${main_pid_line#MainPID=}
    case "\$main_pid" in
        ''|*[!0-9]*) return 1 ;;
    esac
    [ "\$main_pid" -gt 0 ] || return 1
    running_path=\$(readlink -f "/proc/\$main_pid/exe" 2>/dev/null) || return 1
    [ "\$running_path" = "\$expected_path" ]
}

wait_for_release_health() {
    expected_binary=\$1
    url=\$2
    seconds=\$3
    deadline=\$((\$(date +%s) + seconds))
    while [ \$(date +%s) -lt "\$deadline" ]; do
        if release_binary_matches_main_pid "\$expected_binary" \\
            && curl --fail --silent --show-error --connect-timeout 1 --max-time 2 "\$url" >/dev/null; then
            return 0
        fi
        sleep 1
    done
    return 1
}

restore_previous() {
    set +e
    rm -f \"\$deploy_root/.next\"
    if [ -n \"\$previous\" ]; then
        ln -s \"\$previous\" \"\$deploy_root/.rollback\"
        mv -Tf \"\$deploy_root/.rollback\" \"\$deploy_root/current\"
    else
        rm -f \"\$deploy_root/current\"
    fi
    for unit in aio-plugin-supervisor.service aio-idea.service; do
        if [ -f \"\$backup/\$unit.missing\" ]; then
            rm -f \"/etc/systemd/system/\$unit\"
        else
            install -m 0644 \"\$backup/\$unit\" \"/etc/systemd/system/\$unit\"
        fi
    done
    systemctl daemon-reload
    if [ \"\$supervisor_was_enabled\" -eq 0 ]; then
        systemctl disable aio-plugin-supervisor.service
    fi
    if [ \"\$shell_was_enabled\" -eq 0 ]; then
        systemctl disable aio-idea.service
    fi
    if [ -n \"\$previous\" ]; then
        systemctl restart aio-plugin-supervisor.service
        systemctl restart aio-idea.service
        wait_for_release_health "\$previous/aio-idea" http://127.0.0.1:3080/health 300
    else
        systemctl stop aio-idea.service aio-plugin-supervisor.service
    fi
    rm -rf \"\$backup\" \"\$remote_release\"
}

activate_candidate() {
    install -m 0644 \"\$remote_release/systemd/aio-plugin-supervisor.service\" /etc/systemd/system/aio-plugin-supervisor.service \\
        && install -m 0644 \"\$remote_release/systemd/aio-idea.service\" /etc/systemd/system/aio-idea.service \\
        && systemctl daemon-reload \\
        && systemctl enable aio-plugin-supervisor.service aio-idea.service \\
        && ln -s \"\$remote_release\" \"\$deploy_root/.next\" \\
        && mv -Tf \"\$deploy_root/.next\" \"\$deploy_root/current\" \\
        && systemctl restart aio-plugin-supervisor.service \\
        && systemctl restart aio-idea.service \\
        && wait_for_release_health "\$remote_release/aio-idea" http://127.0.0.1:3080/health 300 \\
        && wait_for_health https://aio.addzero.site/health 60
}

if activate_candidate; then
    rm -rf \"\$backup\"
    printf '已激活 %s\\n' '$revision'
else
    restore_previous
    printf '发布失败，已恢复 %s\\n' \"\$previous\" >&2
    exit 1
fi"
