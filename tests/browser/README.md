# 导航浏览器验收

`startup.cjs` 在真实 Web 壳中验证 HTML 快照零额外首屏请求、无快照时的单次请求、304、网络与响应正文超时、重试恢复、焦点事件不打断请求、后台失败保留工作区，以及注销清理。桌面和移动端报告输出到 `target/startup-test/local`，需 Node 可解析 Playwright、parse5 并安装本机 Chrome。

`startup-live.cjs` 使用 `AIO_URL` 和 `AIO_COOKIE_FILE` 测量公网桌面/移动端冷启动及缓存重载，保存 API Server-Timing、资源体积、加载时序和截图到 `AIO_PERFORMANCE_OUTPUT`。不输出 Cookie。

`startup-smoke-live.cjs` 通过公网壳验证 KMP Counter1 的真实 canvas 计数、304 后保留 iframe 和市场 README 图片。只增加当前浏览器内的计数，退出时释放本次挂载。首次 Compose 资源下载允许 120 秒。公网截图使用浏览器合成帧，记录当时实际画面，不等待字体网络请求全部结束。

`verify-delivery-evidence.cjs` 对公网浏览器报告与数据库快照交叉校验，断言三种首次 push 到真实后端调用不超过 60 分钟、激活后 60 秒内开始重新挂载、源码 SHA 一致以及更新期间没有整页导航。

`cleanup-delivery-live.cjs` 通过正式卸载接口清理六个精确命名的临时插件，保留其他安装；数据库清理后以 `verify` 参数再次验证市场中已不存在临时来源。

`cleanup-delivery-repositories.cjs` 使用已授权的 GitHub CLI 删除这六个归档测试仓库，并逐一确认 GitHub 返回 404；凭据需要 `delete_repo` 范围，不读取或输出令牌。

`cleanup-delivery-data.cjs` 在 252 上清理本任务临时仓库的市场、构建记录及按来源和镜像隔离的缓存。必须先通过正式接口卸载，并删除或归档 GitHub 测试仓库以停止发现，脚本只接受 `aio-delivery-e2e-*` 和 `aio-delivery-acceptance-*` 三种语言的精确名称；存在租户绑定或进行中的构建时拒绝清理。

`create-delivery-repositories.cjs` 将 `/tmp/aio-delivery-acceptance-{rust,kotlin,typescript}` 中已由 CLI 生成并提交的干净项目创建为公开仓库，记录首次 push 时间及远端 SHA。`cli-delivery-live.cjs` 通过 `AIO_DELIVERY_SCENARIO=e2e|acceptance` 选择仓库组，`AIO_DELIVERY_LANGUAGE` 可只检查一种语言。`AIO_DELIVERY_OTHER_PLUGIN` 指定 Counter 更新期间应保留实例的另一插件。只读元数据查询在网络失败时最多重试三次。

公网浏览器默认关闭 QUIC，设置 `AIO_BROWSER_HTTP1=1` 可固定 HTTPS/HTTP 1.1 检查链路差异。关闭浏览器前，测试显式释放本次上下文创建的挂载票据，避免反复冷启动耗尽同一用户配额。`compile-failure-live.cjs` 使用临时 TypeScript 仓库的真实编译错误，检查市场保留原版本并继续调用其后端，不调用发布或测试激活接口。

`delivery-live.cjs` 在公网同时保持桌面与移动页面打开，先验证 61 秒无变化轮询，再等待真实源码 push 导致的 Counter 或 README 更新。`AIO_DELIVERY_E2E_MODE=counter|readme` 选择场景；脚本输出 ready 文件后再提交对应源码变更，不调用发布或激活接口。`cli-delivery-live.cjs` 验证三种 CLI 临时插件的真实前后端通信；运行前等待服务器完成首次构建和安装。报告与截图保存到 `target/delivery-test`。

`marketplace.cjs` 使用编译后的市场页面与隔离 HTTP 数据验证桌面/移动端分栏、搜索、键盘选择、README 表格/代码/版本图片、危险链接过滤、直接安装、启停、卸载确认和文档自动更新。报告与截图保存到 `target/marketplace-test`。`AIO_MARKETPLACE_PREVIEW_PORT` 可启动本机预览。公网提交触发自动发布的验收单独记录，不能用协议夹具代替。

`navigation.cjs` 验证场景根筛选、系统树目录及折叠状态、账户全屏页面、返回后页面状态，以及移动端账户菜单关闭抽屉和页面无溢出。使用 Playwright；将其安装到可由 Node 解析的目录后运行：

```bash
AIO_URL=https://aio.addzero.site AIO_COOKIE_FILE=/path/to/session-cookies.txt node tests/browser/navigation.cjs
```

Cookie 文件采用 curl 的 Netscape 格式，仅保存在本机。也可通过 `AIO_ACCOUNT` 和 `AIO_PASSWORD` 验证登录。脚本不会打印 Cookie 或密码；`AIO_SCREENSHOT_DIR` 可指定截图目录，默认为系统临时目录。`AIO_TENANT_ID` 可让用例登录后切到预先准备的隔离租户。

系统页面通过真实接口读取数据。测试只在浏览器收到的目录响应中注入测试工作区、一个状态保持页面和一个社区账户页面，验证运行时页面与全屏入口；这些夹具不会写入数据库，壳没有内置首页。真实二进制发布、Dioxus/Component 调用和市场的停用、启用、卸载确认、重新安装由 `frontend.cjs` 覆盖。

`counter-state.cjs` 同时被 `frontend.cjs` 和线上 `public-admin.cjs` 使用：Dioxus 本地 `+1` 在联网和断网连续点击时均不发请求；真实后端请求暂停期间，本地计数仍即时响应。显式「请求后端 +1」单独验证 Component 和租户透传，不将 UI 事件当成后端事件。

`admin-files.cjs`、`admin-dictionaries.cjs`、`admin-rbac.cjs`、`admin-account.cjs` 通过 `system_management_browser_workflows` 测试启动真实系统插件接口，验证列表、搜索、排序、分页、表单、删除、权限撤销、密码及租户切换。仅允许本机 `aio_keepalive_test` 数据库，测试创建的临时租户由宿主清理；不要对生产环境运行这些写入用例。截图及报告位于 `target/admin-ui-test`。设置 `AIO_ADMIN_PREVIEW_PORT` 可启动隔离开发预览。

`keepalive.cjs` 使用构建后的真实壳与 Compose 前端，在隔离 HTTP 协议夹具中验证桌面/移动端 canvas 绘制、计数状态、A→B→A 的 iframe/JS 实例不变、零重复挂载/释放/资产下载、账户全屏返回、版本替换、LRU 淘汰、页面撤销与会话上下文隔离。夹具不调用生产服务，不替代后端持久化测试。需要 Node 可解析 `playwright`、`pngjs`、`parse5`，以及本机 Chrome：

```bash
dx build --platform web --release --debug-symbols false
AIO_TEST_KMP_FRONTEND=../aio-plugin-kmp-example/dist/frontend node tests/browser/keepalive.cjs
```

截图和测量写到 `target/keepalive-test`。真实 PostgreSQL HTTP 测试还覆盖票据续期、元数据复用，以及权限撤销后带 ETag 的请求仍被拒绝。

设置 `AIO_KEEPALIVE_PREVIEW_PORT=4196` 可保留本地交互预览。此模式使用模拟会话和 API，只验证真实 Compose 绘制及壳导航，不代表实际登录、存储或后端服务。

## Component 市场验收

`component-marketplace.cjs` 上传大屏原生整包，通过市场 UI 安装，导入 CSV、拖拽绑定、保存发布、刷新和手机播放，再验证坏包拒绝并卸载。图表检查读取 canvas 像素，报告保存在 `target/component-delivery/`。`component-preview.cjs` 启动隔离数据库副本，只绑定回环地址。副本准备与正式发布见 `deploy/252/README.md`。运行需要 Playwright、Chrome 和私有 `AIO_COOKIE_FILE`，错误日志不输出 Cookie。

`component-family.cjs` 发布真实 Agent Memory 整包，验证父仓库关系、树层级、折叠展开及父插件未启用时拒绝安装。它不创建占位父包，报告中的 `parentPublished` 明确记录真实 Agent 是否已发布。可用 `AIO_BROWSER_PROXY` 配置浏览器代理；本次公网交付结果见 `deploy/252/component-acceptance.md`。

`plugin-names-live.cjs` 只读检查正式市场的中文插件名与父节点名称，验证桌面、手机的列表和详情，并将截图与结果保存到 `target/component-delivery/chinese-names/`。
