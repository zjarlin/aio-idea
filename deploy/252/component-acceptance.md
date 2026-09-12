# Component 社区交付验收

日期：2026-09-12。正式入口：<https://aio.addzero.site>。

本文保留 2026-09-12 的历史交付记录。Agent 的真实 process 包及 Memory 联合安装已于次日完成，当前状态见 [Agent 与 Memory 正式接入验收](process-acceptance.md)。

## 中文名称更新

2026-09-12 已上线中文名称：计数器示例、任务工作台示例、数据大屏，以及「智能体 → 智能体记忆」。桌面与手机列表、详情和父节点名称通过只读浏览器验收，控制台错误为 0，结果和截图在 `target/component-delivery/chinese-names/`。原有安装保持启用，大屏仍在可安装列表。

此次宿主版本为 `0eb3b2f5a3bc3169d8d9d888d0b471dd4e0d0b6f`。智能体记忆来源为 `fceaccd08cf612c7673216407e52d50172d3f7f5`，新整包内容摘要为 `feb8be45a2fd23c59f3854fee7e58d2b9dc2e366344f2ee2991d7c9710877841`，文件 SHA-256 为 `1ac0cc9386725585746dc5f517bc70dba2493671225671138ba8888bb0dab1d2`。其首次公网请求遇到代理超时，随后经 SSH 管理通道调用同一鉴权发布接口完成，并由公网 API 和浏览器确认。下文保留初次交付的原始版本记录。

## 公网结果

- 数据大屏真实整包已发布；通过市场 UI 安装、运行及卸载，最终留在「可安装」。
- Agent Memory 真实整包已发布；市场按 `aio-plugin-agent → Agent Memory` 展示，可折叠、展开和搜索。父插件未启用时，服务端拒绝安装子插件。
- 安装父插件不会隐式安装子插件；父插件存在已安装子插件时不能停用或卸载。租户隔离、独立可选安装及这些依赖约束已通过真实 PostgreSQL 与 WIT Component 集成测试。
- 既有 Dioxus 和 KMP 示例继续启用，活动版本与发布前一致。
- 真实 Agent 的 process 包尚未发布：宿主还缺少其受限模型出站、数据库角色、密钥注入、受控 Node 子进程及跨插件 broker 接入。当前父节点明确标记「尚未发布」，Agent Memory 因此暂不能在公网安装。测试用父 Component 的通过不代表真实 Agent 联合安装已完成。

## 来源与校验

| 产物 | 来源提交 | v2 内容摘要 | 整包 SHA-256 |
| --- | --- | --- | --- |
| 数据大屏 0.1.0 | `1625ae846ee71e500b6961252d8146f619f9fa08` | `bb9d641da06bb1dbb67daf0b5974cae1c16e367cad09cf1f3c09d69e95c6b0ef` | `6c8a9061743789000d92c21e910b3e7eba34280ff4a96475c06e1c17a3b6821b` |
| Agent Memory 0.1.0 | `bfa6cefd3117f9d651ec41fb4ebe990825184fe1` | `14d2300dd429cd24f3e1c10c633d2f847ed3fe51d0c2f0974df41c8385a16ef3` | `e525967714bf468c6f566e4d94986c752900fea354610bf5f2424fe4751e76db` |

大屏包为 1,406,866 字节，Memory 包为 27,266,322 字节。包位于各自仓库 `dist/`，前后端及迁移由同一摘要覆盖。验收后只提交文档和报告生成器，不改写上述已发布产物。

平台 SDK 与执行槽来源 `cd6ba2336f6af21a7395af9c98c5f1a24bf5df1b`；市场树来源 `9e30813ecf139b36fa352e34e9ba6e2796ea50f2`；控件样式来源 `5b3e4454989d0c1a7afd9538b3efab33812dfd65`。

公网宿主运行版本为 `3e9c92ccd85a045532eb072f24a6a192b4287ab3`，其文件树与验收时主分支 `c88bb0c` 一致，包含原生 Component 发布链和同期首屏样式修复。数据库、密钥与对象目录独立于 release，重启按安装记录恢复执行槽。

## 验证证据

- 大屏核心：契约生成一致性、类型检查、5 项无头编辑引擎测试、6 项 Rust 测试及 WIT 文档测试。
- 大屏真实运行：17 项 Wasmtime、PostgreSQL、对象存储场景，覆盖冲突保存、发布与草稿隔离、回退、重启、租户和权限、引用完整性及无效包保护。
- 大屏本地浏览器：14 项流程，含真实鼠标拖拽、缩放、旋转、组合与撤销，CSV 导入、单元格编辑、筛选映射和手机播放；3 套模板及 8 种图表完成截图与像素检查。
- 宿主：52 项常规服务端测试通过；3 项 Component 集成测试在独立 PostgreSQL 上另行通过，包括安装记录与执行槽在中断后的恢复、父子依赖、权限隔离及失败回退。Web 检查、Web release 构建与 glibc 2.17 服务端构建通过。
- 公网大屏：完整上传、下载字节一致、UI 安装、CSV 导入、拖入柱图、绑定字段、保存发布、刷新恢复、无效包拒绝和 UI 卸载通过。桌面与手机图表均检测到 3,766 个着色采样点、54 种颜色，控制台错误为 0。
- 公网插件树：Agent Memory 为第二层节点，折叠展开通过，未启用父插件时安装被拒绝；`parentPublished=false`、控制台错误为 0。

宿主原始报告及截图位于 `target/component-delivery/public/`：`report.json`、`family-report.json`、`final-state.json`、`marketplace-overview.png`、`plugin-family.png`、`screen-desktop.png`、`screen-mobile.png`。大屏汇总报告位于同级 `aio-plugin-screen/dist/acceptance-report.md` 和 JSON 文件，生成器核对整包摘要及运行测试摘要一致。

公网测试只创建带本次唯一名称的临时大屏与数据集，结束后清理并卸载本次安装。Cookie 仅从本机私有文件读取，报告和 Git 不保存凭据。发布不会自动安装，本次未开启源码推送后的自动跟随发布。

## 数据保护

上线前已备份宿主数据库并在独立本机数据库还原验收。备份文件为本机 `target/component-delivery/database-1789209257141.dump`，174,687,128 字节，SHA-256 为 `a092a2afec3e41dbf6689746dee91aab44f3478049f658f91e6dc07671af32fc`，不进入 Git。

原生 Component 使用独立 `aio_plugin_components` 数据库，已撤销 public schema 的 PUBLIC 权限。持久密钥和对象位于 `/opt/aio-idea/component-storage`；卸载不删除插件业务数据，发布失败不替换当前安装。
