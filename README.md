# aio-idea

这是基于 [aio-platform](https://github.com/zjarlin/aio-platform) 组装的应用产品和官方插件中心。系统引导能力来自 Cargo 中锁定完整提交 SHA 的独立插件仓库；`aio.toml` 定义默认租户首次启动时安装的运行时 Git 组合。活动版本、租户绑定、市场元数据和完整 `.aio-plugin` 二进制包保存在 PostgreSQL，本地版本目录只是可恢复的运行缓存。

开发者在本地完成构建与 `aio plugin package` 后，使用来源绑定凭证执行 `aio plugin publish` 直接上传二进制包。发布不依赖 GitHub Actions，不要求提交编译产物；验证与健康检查成功后在线激活，失败保留旧版本。成功发布的二进制包可以下载、在其他租户安装和回滚，无需回连 Git。当前动态目标为 PageDefinition、Wasm Component 和隔离 process；Dioxus 原生源码插件仍通过整体构建装配。

当前包协议为格式 2，CLI 与宿主使用同一固定提交的共享验证库。协议已定义前后端联合包，但本宿主尚未接通前端隔离挂载和通信桥，因此会在暂存前拒绝包含 `plugin.frontend` 的包，不会把安装成功后只能显示空白的前端加入市场。Dioxus 联合包在线运行仍待完成。

插件开发先让 AI 阅读 aio-platform 仓库的 `aio-plugin-development` Skill 和对应语言规约。前端、后端、共享逻辑和子插件属于同一个功能仓库。

顶部场景选择当前菜单树的根，侧栏只显示当前场景的业务菜单。账户插件贡献的个人资料、设置、市场和租户切换页面从左下角进入独立全屏视图，点击“返回主后台”后保留原场景、页面及页面内部状态；账户页面不会重复出现在侧栏。此规则由共享壳处理，也适用于运行时子插件贡献的账户页面。

```bash
dx serve
cargo run --no-default-features --features desktop
cargo run --no-default-features --features server
aio plugin install <git>
aio plugin sync
```

252 生产发布使用 [发布脚本和服务单元](deploy/252/README.md)，由独立进程监督器管理隔离容器，Cloudflare Tunnel 或反向代理提供公网入口。`compose.yaml` 仅是应用容器构建入口，尚未装配监督器 socket、共享缓存和受限运行网络，不能单独作为完整插件宿主启动。
