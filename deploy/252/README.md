# 252 公网发布

此目录记录 `aio.addzero.site` 在 252 主机的运行单元。应用监听 `127.0.0.1:3080`，独立 Cloudflare Tunnel `4be41351-a4ef-4ce2-9ef2-f573946fcfcf` 提供 TLS 公网入口。

发布物按提交放入 `/opt/aio-public-shell/releases/<revision>`，健康检查通过后原子更新 `/opt/aio-public-shell/current`。失败时保留旧软链接和旧发布目录。

```bash
systemctl restart aio-public-shell.service
curl --fail http://127.0.0.1:3080/health
curl --fail http://127.0.0.1:3080/api/plugins/aio-plugin-hello/health
systemctl restart aio-public-shell-tunnel.service
curl --fail https://aio.addzero.site/health
```

`credentials.json` 是服务器私密文件，不进入 Git。
