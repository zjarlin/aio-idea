# 252 公网发布

此目录记录 `aio.addzero.site` 在 252 主机的运行单元。应用监听 `127.0.0.1:3080`，独立 Cloudflare Tunnel `4be41351-a4ef-4ce2-9ef2-f573946fcfcf` 提供 TLS 公网入口。

发布物按提交放入 `/opt/aio-idea/releases/<revision>`，健康检查通过后原子更新 `/opt/aio-idea/current`。失败时保留旧软链接和旧发布目录。

252 使用 glibc 2.17，服务端必须显式构建为对应 Zig 目标：

```bash
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17 --no-default-features --features server
```

```bash
systemctl enable --now aio-plugin-supervisor.service
systemctl restart aio-idea.service
curl --fail http://127.0.0.1:3080/health
systemctl restart aio-idea-tunnel.service
curl --fail https://aio.addzero.site/health
```

每个发布目录必须同时包含服务端二进制、`web/` 和 `aio.toml`。进程插件由 root 监督器在无外网的独立容器中运行，主壳只通过 Unix socket 调用监督器，不加入 Docker 用户组。Wasm Component 在宿主重启时按租户恢复，首次启动窗口配置为 300 秒以容纳跨语言 Component 冷启动。文件插件把内容写入发布目录之外的 `/opt/aio-idea/file-storage`，发布脚本负责创建并授权给 `aio-shell`；PostgreSQL 保存文件元数据。`credentials.json` 和只包含数据库连接、初始管理员密码的 `/opt/aio-idea/runtime.env` 是服务器私密文件，不进入 Git。首次启动必须设置 `AIO_BOOTSTRAP_PASSWORD`。发布凭据只能由 `AIO_PLUGIN_PUBLISH_ACCOUNTS` 明确列出的平台账号创建或撤销；普通租户管理员仍可安装和管理租户插件，但不能认领全局 Git 发布来源。

## 原生 v2 Process

Agent 使用原生 v2 整包中的 Linux ELF 与 Compose 前端，Pi SDK 依赖由预置 Node 镜像提供。宿主和监督器共同读取私有 `/opt/aio-idea/process.env`，以 `AIO_PROCESS_ROOT` 保存运行授权及 socket；加密主密钥仍由 Component keyring 管理。模型只能通过宿主 broker 访问清单及宿主共同批准的 HTTPS 基址，容器自身使用 `--network=none`。

上线前运行 `component-storage.cjs backup` 和 `backup-components`，同时备份宿主数据库、插件数据库和宿主密钥目录。构建 Agent 仓库的 `Containerfile` 中 `runtime` 目标并核对不可变镜像 ID，然后设置 `AIO_PROCESS_IMAGE=sha256:...` 执行 `node deploy/252/component-storage.cjs processes`，登记预置镜像、模型地址和持久目录。模型未配置时仍接收加密资料，整理任务等待空间绑定模型。

`process-rehearsal.cjs` 在 252 的 `/opt/aio-idea/process-test-20260913` 使用独立 PostgreSQL 集群恢复 `snapshot.dump`，避免同集群数据库角色名与生产冲突。`prepare` 恢复副本并停用复制的安装和发布任务；`start` 启动该目录里的候选二进制与独立监督器；`stop` 只终止该目录的进程。宿主使用 4245 回环端口。新旧监督器均校验自身目录归属，演练清理不会停止生产插件。

通过演练后按下节发布宿主，再上传真实 Agent 整包，先安装父插件智能体，再安装智能体记忆。`tests/browser/agent-process.cjs publish` 执行发布和安装，默认命令验证对话与图谱，`resume` 复用同一批资料继续验证；它本身不执行重启。设置 `AIO_AGENT_TEST_CLEANUP=1` 删除验收来源和会话；保留两个正式插件的安装。

## Rust Source 发布

`rust-source` 会进入同一个 Rust 进程的 Dill 图，因此不能按租户在线替换。它和壳一起作为一个发布单元：先由受控发布端在隔离 worktree 中编译，随后原子切换整个版本。

```bash
./deploy/252/publish.zsh
./deploy/252/publish.zsh <完整 Git SHA>
```

发布器拒绝未提交的工作树和非完整 SHA。它会在本地分别执行服务端测试、Web 检查、glibc 2.17 服务端构建和 Web 构建，将候选二进制、前端资源、`aio.toml` 和两项 systemd 单元上传到远端临时目录。切换前会备份现有 unit 与 enabled 状态；候选服务只有在 `aio-idea.service` 的 `MainPID` 确实执行当前 release 二进制后，本机 `/health` 才会被接受，随后还必须在 60 秒内通过公网健康检查。任一步失败都会恢复旧链接、unit、enabled 状态和服务，并删除失败发布目录。

可以用绝对路径 `AIO_DEPLOY_TARGET_DIR` 复用本机 Cargo 编译缓存；源码仍来自完整 SHA 的隔离 worktree，所有测试及构建照常执行。

本机与 252 均需安装 `rsync`。传输按内容校验，并压缩发送变更文件；与当前版本内容相同的资产通过硬链接复用，新文件仍落入独立候选目录。后续只切换目录链接，不原地改写已发布文件。

## 原生 v2 Component

`component-storage.cjs backup` 保存宿主 PostgreSQL 的 custom 格式备份及 SHA-256；`rehearse` 将其还原到独立本机数据库，并停用副本里的旧插件和发布任务。`tests/browser/component-preview.cjs` 只接受该副本，开发身份来自已有登录会话。

首次上线先完成副本验收，再运行 `component-storage.cjs provision` 创建专用数据库 `aio_plugin_components`、撤销 PUBLIC schema 权限、备份并更新服务器环境文件。密钥和对象保存在 `/opt/aio-idea/component-storage`，发布和回退不删除此目录。纯 Component 环境没有启用进程插件时，不要求 Docker 监督器在线。

原生整包上传到 `POST /api/runtime/components/publish`，类型为 `application/vnd.aio.component+gzip`，沿用平台来源发布者身份。通过真实 WIT、Wasmtime、迁移及健康检查后出现在可安装列表，发布操作不会自动安装。子插件通过清单 `plugin.marketplace.parent` 绑定父仓库；必须先启用同租户父插件，子插件由用户独立选择安装，卸载父插件前须卸载子插件。

浏览器验收：先构建服务端与 Web，启动副本宿主，再设置 `NODE_PATH` 为已安装 Playwright 的依赖目录、`AIO_COOKIE_FILE` 为有效会话 Cookie 文件，运行 `node tests/browser/component-marketplace.cjs`。报告和桌面、手机截图位于 `target/component-delivery/rehearsal/`。`AIO_URL` 可以指向正式宿主；正式验收最后卸载本次安装，使大屏保留在可安装列表。测试失败信息不输出 Cookie。

2026-09-12 的正式发布、整包摘要、测试结果及 Agent 运行边界见 [Component 交付验收](component-acceptance.md)。

这条路径是受控发布器，不是公网 Git 安装器。公网运行时只安装已构建的 `wasm-component`、`page-definition` 与受限 `process` 产物，安装过程不会执行仓库脚本。CI 同时上传原始 Git commit 和所需 tree 对象；服务端离线校验对象哈希及清单、artifact 的提交归属，无需为发布回连远程 Git。健康检查和激活成功后才更新数据库市场条目。
