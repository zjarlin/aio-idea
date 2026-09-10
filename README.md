# AIO

这是 AIO 的公网宿主组合。系统引导能力来自 Cargo 中锁定完整提交 SHA 的独立 Git 插件；`aio.toml` 定义默认租户首次启动时安装的运行时 Git 组合，活动版本和租户绑定正式保存在 PostgreSQL。当前默认运行时插件是 [aio-plugin-hello](https://github.com/zjarlin/aio-plugin-hello)。

插件开发先让 AI 阅读 AIO 仓库的 `aio-plugin-development` Skill 和对应语言规约。

顶部场景选择当前菜单树的根，侧栏只显示当前场景的业务菜单。账户插件贡献的个人资料、设置、市场和租户切换页面从左下角进入独立全屏视图，点击“返回主后台”后保留原场景、页面及页面内部状态；账户页面不会重复出现在侧栏。此规则由共享壳处理，也适用于运行时子插件贡献的账户页面。

```bash
dx serve
cargo run --no-default-features --features desktop
cargo run --no-default-features --features server
aio plugin install <git>
aio plugin sync
```

生产发布使用 `docker compose up -d --build`；容器只暴露本机 `127.0.0.1:3080`，由 Cloudflare Tunnel 或反向代理提供公网入口。
