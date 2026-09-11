# 导航浏览器验收

`navigation.cjs` 验证场景根筛选、系统树目录及折叠状态、账户全屏页面、返回后页面状态，以及移动端账户菜单关闭抽屉和页面无溢出。使用 Playwright；将其安装到可由 Node 解析的目录后运行：

```bash
AIO_URL=https://aio.addzero.site AIO_COOKIE_FILE=/path/to/session-cookies.txt node tests/browser/navigation.cjs
```

Cookie 文件采用 curl 的 Netscape 格式，仅保存在本机。也可通过 `AIO_ACCOUNT` 和 `AIO_PASSWORD` 验证登录。脚本不会打印 Cookie 或密码；`AIO_SCREENSHOT_DIR` 可指定截图目录，默认为系统临时目录。`AIO_TENANT_ID` 可让用例登录后切到预先准备的隔离租户。

系统页面通过真实接口读取数据。测试只在浏览器收到的目录响应中注入测试工作区、一个状态保持页面和一个社区账户页面，验证运行时页面与全屏入口；这些夹具不会写入数据库，壳没有内置首页。真实二进制发布、Dioxus/Component 调用和市场的停用、启用、卸载确认、重新安装由 `frontend.cjs` 覆盖。

`admin-files.cjs`、`admin-dictionaries.cjs`、`admin-rbac.cjs`、`admin-account.cjs` 通过 `system_management_browser_workflows` 测试启动真实系统插件接口，验证列表、搜索、排序、分页、表单、删除、权限撤销、密码及租户切换。仅允许本机 `aio_keepalive_test` 数据库，测试创建的临时租户由宿主清理；不要对生产环境运行这些写入用例。截图及报告位于 `target/admin-ui-test`。设置 `AIO_ADMIN_PREVIEW_PORT` 可启动隔离开发预览。

`keepalive.cjs` 使用构建后的真实壳与 Compose 前端，在隔离 HTTP 协议夹具中验证桌面/移动端 canvas 绘制、计数状态、A→B→A 的 iframe/JS 实例不变、零重复挂载/释放/资产下载、账户全屏返回、版本替换、LRU 淘汰、页面撤销与会话上下文隔离。夹具不调用生产服务，不替代后端持久化测试。需要 Node 可解析 `playwright`、`pngjs`、`parse5`，以及本机 Chrome：

```bash
dx build --platform web --release --debug-symbols false
AIO_TEST_KMP_FRONTEND=../aio-plugin-kmp-example/dist/frontend node tests/browser/keepalive.cjs
```

截图和测量写到 `target/keepalive-test`。真实 PostgreSQL HTTP 测试还覆盖票据续期、元数据复用，以及权限撤销后带 ETag 的请求仍被拒绝。

设置 `AIO_KEEPALIVE_PREVIEW_PORT=4196` 可保留本地交互预览。此模式使用模拟会话和 API，只验证真实 Compose 绘制及壳导航，不代表实际登录、存储或后端服务。
