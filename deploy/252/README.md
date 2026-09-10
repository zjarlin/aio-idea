# 252 公网发布

此目录记录 `aio.addzero.site` 在 252 主机的运行单元。应用监听 `127.0.0.1:3080`，独立 Cloudflare Tunnel `4be41351-a4ef-4ce2-9ef2-f573946fcfcf` 提供 TLS 公网入口。

发布物按提交放入 `/opt/aio-public-shell/releases/<revision>`，健康检查通过后原子更新 `/opt/aio-public-shell/current`。失败时保留旧软链接和旧发布目录。

252 使用 glibc 2.17，服务端必须显式构建为对应 Zig 目标：

```bash
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17 --no-default-features --features server
```

```bash
systemctl enable --now aio-plugin-supervisor.service
systemctl restart aio-public-shell.service
curl --fail http://127.0.0.1:3080/health
systemctl restart aio-public-shell-tunnel.service
curl --fail https://aio.addzero.site/health
```

每个发布目录必须同时包含服务端二进制、`web/` 和 `aio.toml`。进程插件由 root 监督器在无外网的独立容器中运行，主壳只通过 Unix socket 调用监督器，不加入 Docker 用户组。Wasm Component 在宿主重启时按租户恢复，首次启动窗口配置为 300 秒以容纳跨语言 Component 冷启动。`credentials.json` 和只包含数据库连接、初始管理员密码的 `/opt/aio-public-shell/runtime.env` 是服务器私密文件，不进入 Git。首次启动必须设置 `AIO_BOOTSTRAP_PASSWORD`。发布凭据只能由 `AIO_PLUGIN_PUBLISH_ACCOUNTS` 明确列出的平台账号创建或撤销；普通租户管理员仍可安装和管理租户插件，但不能认领全局 Git 发布来源。

## Rust Source 发布

`rust-source` 会进入同一个 Rust 进程的 Dill 图，因此不能按租户在线替换。它和壳一起作为一个发布单元：先由受控发布端在隔离 worktree 中编译，随后原子切换整个版本。

```bash
./deploy/252/publish.zsh
./deploy/252/publish.zsh <完整 Git SHA>
```

发布器拒绝未提交的工作树和非完整 SHA。它会在本地分别执行服务端测试、Web 检查、glibc 2.17 服务端构建和 Web 构建，将候选二进制、前端资源、`aio.toml` 和两项 systemd 单元上传到远端临时目录。切换前会备份现有 unit 与 enabled 状态；候选服务在 300 秒内通过本机健康检查、并在 60 秒内通过公网健康检查后才算激活。任一步失败都会恢复旧链接、unit、enabled 状态和服务，并删除失败发布目录。

这条路径是受控发布器，不是公网 Git 安装器。公网运行时只安装已构建的 `wasm-component`、`page-definition` 与受限 `process` 产物，安装过程不会执行仓库脚本。CI 同时上传原始 Git commit 和所需 tree 对象；服务端离线校验对象哈希及清单、artifact 的提交归属，无需为发布回连远程 Git。健康检查和激活成功后才更新数据库市场条目。
