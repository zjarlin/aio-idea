# 导航浏览器验收

`navigation.cjs` 验证场景根筛选、系统树目录及折叠状态、账户全屏页面、返回后页面状态，以及移动端账户菜单关闭抽屉和页面无溢出。使用 Playwright；将其安装到可由 Node 解析的目录后运行：

```bash
AIO_URL=https://aio.addzero.site AIO_COOKIE_FILE=/path/to/session-cookies.txt node tests/browser/navigation.cjs
```

Cookie 文件采用 curl 的 Netscape 格式，仅保存在本机。也可通过 `AIO_ACCOUNT` 和 `AIO_PASSWORD` 验证登录。脚本不会打印 Cookie 或密码；`AIO_SCREENSHOT_DIR` 可指定截图目录，默认为系统临时目录。`AIO_TENANT_ID` 可让用例登录后切到预先准备的隔离租户。

系统页面通过真实接口读取数据。测试只在浏览器收到的目录响应中注入 Hello、一个状态保持页面和一个社区账户页面，验证运行时页面与全屏入口；这些夹具不会写入数据库，也不代表实际安装发布验证。真实二进制发布与 Dioxus/Component 调用由 `frontend.cjs` 覆盖。
