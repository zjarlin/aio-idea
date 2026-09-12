# 公网宿主服务

Web 发布需要 Node.js 22.12+。`npm ci --prefix deploy --ignore-scripts` 安装锁定的 HTML 解析器，`node deploy/prepare-web.cjs` 为新构建添加 Wasm preload 并生成静态资源 gzip 副本。必须在合并历史资源前执行；252 发布脚本已包含这两步。

`aio-plugin-supervisor.service` 使用与公网壳相同的发布二进制启动 root 隔离监督器。`252/aio-idea.service` 等待监督器就绪，并且只能通过 `/run/aio-plugin-supervisor/supervisor.sock` 请求受限生命周期操作，不获得 Docker socket 权限。

发布时将两份 unit 一起安装到 `/etc/systemd/system`，执行 `systemctl daemon-reload` 后启用并启动监督器，再重启公网壳。
