# AIO

这是 AIO 的公网宿主组合。系统引导能力来自 Cargo 中锁定完整提交 SHA 的独立 Git 插件；`aio.toml` 定义默认租户首次启动时安装的运行时 Git 组合，活动版本和租户绑定正式保存在 PostgreSQL。当前默认运行时插件是 [aio-plugin-hello](https://github.com/zjarlin/aio-plugin-hello)。

插件开发先让 AI 阅读 AIO 仓库的 `aio-plugin-development` Skill 和对应语言规约。

```bash
dx serve
cargo run --no-default-features --features desktop
cargo run --no-default-features --features server
aio plugin install <git>
aio plugin sync
```

生产发布使用 `docker compose up -d --build`；容器只暴露本机 `127.0.0.1:3080`，由 Cloudflare Tunnel 或反向代理提供公网入口。
