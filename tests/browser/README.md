# 导航浏览器验收

`navigation.cjs` 验证场景根筛选、账户全屏页面、返回后页面状态，以及移动端账户菜单关闭抽屉和页面无溢出。使用 Playwright；将其安装到可由 Node 解析的目录后运行：

```bash
AIO_URL=https://aio.addzero.site AIO_COOKIE_FILE=/path/to/session-cookies.txt node tests/browser/navigation.cjs
```

Cookie 文件采用 curl 的 Netscape 格式，仅保存在本机。也可通过 `AIO_ACCOUNT` 和 `AIO_PASSWORD` 验证登录。脚本不会打印 Cookie 或密码；`AIO_SCREENSHOT_DIR` 可指定截图目录，默认为系统临时目录。测试账号需要查看系统和社区场景的权限，当前租户需要安装带前端计数器的 KMP 示例插件。
