# 公网宿主服务

`aio-plugin-supervisor.service` 使用与公网壳相同的发布二进制启动 root 隔离监督器。`252/aio-public-shell.service` 等待监督器就绪，并且只能通过 `/run/aio-plugin-supervisor/supervisor.sock` 请求受限生命周期操作，不获得 Docker socket 权限。

发布时将两份 unit 一起安装到 `/etc/systemd/system`，执行 `systemctl daemon-reload` 后启用并启动监督器，再重启公网壳。
