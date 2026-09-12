# 全栈插件自动发布验收记录

验收地址：<https://aio.addzero.site/>。2026-09-12 公网功能验收通过，最终保留 `KMP Counter1`。GitHub 临时仓库已归档，删除仍待 CLI 设备授权；市场、运行实例与测试数据已清理。

## 已部署能力

- CLI 默认生成 Rust、Kotlin、TypeScript 全栈插件，生成发布标记、固定工具链、共享模型、测试及 README；系统源码插件使用 `--kind system`。
- 宿主每 5 分钟发现 `zjarlin` 的公开、非 fork、非归档且有发布标记的仓库，每 60 秒检查已发现来源。默认分支 push 进入 PostgreSQL 构建队列，不依赖 GitHub Actions。
- 252 的单工作进程使用固定摘要镜像，限制每个源码任务 4 CPU、8 GiB、45 分钟。源码容器不持有发布或数据库凭据；发布、文档保存及租户切换在宿主执行。
- 首次发布安装到 `default` 租户，其他已安装租户由持久化任务升级；失败保留旧版。停用、卸载和回滚规则由真实 PostgreSQL HTTP 测试覆盖。
- 市场使用共享组件实现分组列表与 README 详情、搜索、键盘选择、分栏调整、移动返回和管理操作。文档及本地图片绑定成功发布的完整源码 SHA，浏览时不读取移动分支。
- 壳每 30 秒检查目录，仅重新挂载版本变化的插件；导航片段经统一桥保存。

## 公网验收

| 验收项目 | 状态 | 证据 |
| --- | --- | --- |
| 桌面与手机市场、README 图片、直接安装及管理操作 | 通过 | `target/delivery-test/marketplace-live-report.json` 与两张 `*-marketplace-live.png` |
| 手机长版本号与摘要不溢出 | 通过 | 共享 UI `3ef0d45`，详情容器宽度断言及手机截图 |
| Rust 试运行插件真实 Component 调用 | 通过 | `rust-cli-live-report.json` |
| TypeScript 试运行插件真实 Node 调用 | 通过 | `e2e-typescript-cli-live-report.json` |
| 三种全新 CLI 仓库首次 push 后自动安装与真实前后端调用 | 通过 | `acceptance-pushes.json`、`acceptance-timing-report.json`、`acceptance-all-cli-live-report.json` |
| 桌面与手机原页面自动出现 `KMP Counter1` | 通过 | `counter-report.json`，两个页面更新期间整页导航均为 0；Counter 路由和其他插件实例保持，点击后各有 2756 个 canvas 像素改变 |
| README 单独提交后原详情自动更新 | 通过 | `readme-report.json`、`readme-push.json`；提交只改 README，文档与包版本对应，选中项和阅读位置保持 |
| 真实编译失败保留旧版及下一次提交恢复 | 通过 | `compile-failure-report.json`：任务 14 的 TS2322 不影响原包，真实后端返回 value=3；任务 15 随新提交自动恢复 |
| 临时安装、版本包、市场及构建数据清理 | 通过 | 六个来源、七个版本包及六个来源缓存已清理；`cleanup-market-report.json` 中仅余两个正式插件 |
| GitHub 临时仓库删除 | 待授权 | 六个仓库均已归档，`archived-repositories.json`；当前 CLI token 缺少 `delete_repo` 范围 |

## 提交与计时

`verify-delivery-evidence.cjs` 交叉检查 push 记录、数据库激活事件、前端挂载时间、后端响应和源码 SHA；输出 `acceptance-summary.json`。所有最终首次发布及更新均满足 push 后 60 分钟上限。

| 场景 | 源码 SHA | 构建任务 | 包版本 SHA256 | 结果 |
| --- | --- | --- | --- | --- |
| Rust CLI 首次 push | `4f262a7913f57d6b74d2db31d3d32a4ae1edd21a` | 13 | `ee644d34359771d1c8d28b92834b508aea8d6d1c4cc2ffe166a77124522d2f07` | 约 33 分钟自动安装；47.1 分钟内确认真实 Component 调用 |
| Kotlin CLI 首次 push | `1445a6586a9d58787321fb3b83448f69252215e7` | 11 | `257d8de180fbce3510b33ae17f119b602f566d51bd65e354ef6c69f7ce3bd4a0` | 约 20.5 分钟自动安装；46.5 分钟内确认真实 Ktor 调用 |
| TypeScript CLI 首次 push | `0356917765c99ddda61a51b80b3a65ad8f822049` | 12 | `71ffab1d742c193896cc399c1001e19b4ddaf7e1116fbf6bdc6511e56cfde699` | 约 21.3 分钟自动安装；45.8 分钟内确认真实 Node 调用 |
| Counter 文案 | `bdc2ccf830a4c58183654553279a42cf7cf63b51` | 18 | `0d82187caa5c909c78e5af0a06a309fecb7024dbeed957bd3358f5d2f126e5bd` | 桌面/手机激活后 24.428 / 21.278 秒开始切换；push 后 169.275 / 189.475 秒完成文案与计数检查 |
| README 单独提交 | `581f9d782337b18c7099c0565acb4aa5ccebed05` | 19 | `68e361c7d09c83bd1f24976003159d8e26c36165b8285abbb030d7dfad527700` | 桌面/手机原详情在 push 后 142.140 / 143.047 秒更新 |

任务 18、19 均通过 252 独立网络出口获取源码及构建，未调用人工发布、测试激活或更新接口。浏览器始终保留原页面；三种首次发布用例只创建并 push 仓库，未配置逐仓凭据或 GitHub Actions。

## 已完成的底层验证

隔离 PostgreSQL HTTP 测试通过，覆盖领取、续租、上传、发布、网络退避、其他来源不被阻塞、首次安装、多个租户升级、单租户失败重试、停用与卸载不复活、手动回滚保持到下一次成功发布、旧任务不能覆盖新目标，以及无变化轮询不再激活。

CLI 单元测试、三种模板构建和 Kotlin 共享模型测试已执行。Kotlin 生成共享模块 Java 17 字节码和服务 Java 21 字节码，构建 JDK 固定为 25，服务仍使用 JRE 21。

市场隔离浏览器测试覆盖 README 标题、表格、代码、相对图片、危险 HTML/链接过滤、搜索、键盘导航、移动返回和文档轮询。壳的隔离浏览器测试使用真实 Compose 产物检查 canvas 像素、计数、页面保持及版本替换；最新部署的桌面/手机保活测试及 10 项资源缓存测试均通过。协议夹具测试与上述真实公网用例分别保留证据。

构建重启恢复的实际记录：KMP 任务 8 在工作进程重启后继续处理相同源码 `1ef261c221d75338382a53d86cd8eac49cadf1a5`，发布包 `22ebf21f4c79a90aa26b941d09141583342a5f525185bb8911afe63f86e6f7bd`，随后 `default` 与另一已安装租户自动激活。前后数据库快照保存在 `pre-kotlin-tools-restart.json` 和 `post-kotlin-tools-restart.json`。

## 运行条件与问题记录

首次冷启动曾因 GitHub 下载、未预装的工具链和 TypeScript 类型配置失败，初轮提交未满足 60 分钟上限。这些失败保留在任务历史中；最终首次发布测试使用修复后 CLI 新建的三个公开仓库，单独计时。

Rust 镜像已固定预装 Dioxus、wasm-bindgen、wasm-tools、Binaryen 和 esbuild。Kotlin 镜像预装固定 Kotlin CLI、Node、pnpm，并通过断网工具链检查，避免任务再次下载大型工具归档。

252 直连 GitHub 的固定 IP 路径不稳定，现已部署独立的 `aio-egress.service`。构建容器通过 `http://172.17.0.1:17892` 访问此出口，无需 Mac 在线；上游凭据仅保存在服务器专用账户可读的私有配置中，不进入构建容器或仓库。服务使用校验 SHA256 的官方固定发行包，仅绑定 Docker 网桥，配置和安装说明位于平台仓库 `delivery/egress`。

公网自动化浏览器曾遇到 QUIC 中断、脚本初始化前点击以及浏览器强制关闭留下挂载票据的问题。测试现已等待事件绑定，并在关闭上下文前显式释放自身创建的票据；不改变用户真实页面的挂载配额。最终公网用例固定 HTTPS/HTTP 1.1。产品的资源传输使用独立超时及最多三次网络重试，业务请求超时和资源完整性检查保持有效。

首轮 Counter 更新已在两个页面显示成功，但测试错误地假设初始整页导航计数必为 1，导致用例失败。修正为更新前后导航差值与壳实例标记后，重新发布旧文案基线，再执行真实 Counter1 push 并通过；首轮证据保留在 `counter-first-attempt`，正式结论使用任务 18。

## 部署对应

- 服务端及静态前端来自宿主 `33a32119c69edd0003e09d404fa15e3c36a2023d`，服务端二进制 SHA256 为 `993a3fee58c241ea50e1d2e8395e47525bb56d23885b1f1e0b008426036bd34f`；共享 UI 为 `3ef0d45`。于 16:50 CST 完成部署，正式 Counter 和 README 验收期间没有重启宿主。
- 构建工作进程增加无凭据代理支持，二进制 SHA256 为 `6c23a17af9685f4a0b5f00f13c507948f50edda00cd38489f0634ac404f2c531`。
- 市场项目已部署提交 `512c5a65beef5062c91859cdb9b765694a53d4fd`。
- CLI 已安装到本机和 252，252 二进制 SHA256 为 `e8cd0b29bb379d3c850763efafcc06108f8587edd67019d2ef9a3d6cce86e9ab`。
- 独立网络出口安装代码已推送平台提交 `e6bda7a`。宿主之后的提交只补充验收脚本与报告，不要求再次部署产品。

全部浏览器截图及机器可读报告位于宿主仓库的 `target/delivery-test`。最终市场截图为 `desktop-final-market.png`、`mobile-final-market.png` 和 `mobile-final-market-list.png`；Counter 截图为 `desktop-counter-after.png`、`mobile-counter-after.png`。源码、任务、包版本与浏览器时间通过 `acceptance-summary.json` 对应。
