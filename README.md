# AIO

这是 AIO 的公网宿主组合。页面和后端能力来自 `aio.toml` 中配置的 Git 插件，`.aio/plugins.lock` 锁定实际提交。当前内置市场示例是 [aio-plugin-hello](https://github.com/zjarlin/aio-plugin-hello)。

插件开发先让 AI 阅读 AIO 仓库的 `aio-plugin-development` Skill 和对应语言规约。

```bash
dx serve
cargo run --no-default-features --features desktop
cargo run --no-default-features --features server
aio plugin install <git>
aio plugin sync
```

生产发布使用 `docker compose up -d --build`；容器只暴露本机 `127.0.0.1:3080`，由 Cloudflare Tunnel 或反向代理提供公网入口。
