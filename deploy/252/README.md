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

每个发布目录必须同时包含服务端二进制、`web/` 和 `aio.toml`。进程插件由 root 监督器在无外网的独立容器中运行，主壳只通过 Unix socket 调用监督器，不加入 Docker 用户组。Wasm Component 在宿主重启时按租户恢复，首次启动窗口配置为 300 秒以容纳跨语言 Component 冷启动。`credentials.json` 和只包含数据库连接、初始管理员密码的 `/opt/aio-public-shell/runtime.env` 是服务器私密文件，不进入 Git。首次启动必须设置 `AIO_BOOTSTRAP_PASSWORD`。
