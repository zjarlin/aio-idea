# 直接注册验收

2026-09-13，`https://aio.addzero.site` 已激活宿主 `2acb437dfc256902128a7bdc5cf54d0ecd66eb89`。

登录页通过「注册账号」打开统一弹窗，只填写账号、密码和确认密码。不要求验证码、邮箱验证或审核；成功后自动登录并创建个人工作区。沿用现有密码最短长度规则，当前默认 12 个字符。账号重名返回 409，输入内容保留。

账号、工作区、成员、管理员权限及会话在同一 PostgreSQL 事务提交。新账号只获得自己工作区的管理权限，不能切换至默认工作区或发布平台插件。密码使用 Argon2 摘要，会话沿用 HttpOnly Cookie。

## 验证结果

- 身份插件工作区测试 3 项通过，包括真实 PostgreSQL 注册测试：并发重名、输入错误、请求大小、伪造权限、事务完整性、工作区隔离、会话恢复及再次登录。
- 宿主服务端 53 项通过，9 项需要各自独立环境的其他集成测试未在本次发布重复运行；Web 检查及 Linux、Web 发布构建通过。
- Playwright 在本机和正式域名分别完成 1440×1000、390×844 注册流程：取消卸载、密码确认、注册即登录、重名错误、刷新恢复、退出和重登、租户隔离及文件和权限接口访问。
- 桌面、手机弹窗无溢出，截图已人工检查，无 JavaScript 页面异常。正式环境原有账号会话仍有效。
- 本机共 4 个、正式环境 2 个临时账号及其工作区均已按精确 ID 清理。本机重跑前的 2 个账号来自验收脚本响应读取问题。

报告与截图：`target/registration-test/local/`、`target/registration-test/live/`。报告不包含密码或 Cookie。复跑方法见 `tests/browser/README.md`。本机真实 PostgreSQL 预览位于 `http://127.0.0.1:4216`。

## 来源

| 仓库 | 提交 |
| --- | --- |
| aio-plugin-identity | `340b5f2f3956100a32ecbbdcee5cccb6dc84c9dd` |
| aio-plugin-rbac | `7c954695525197dab0892e8b6e7e646d6e2bb596` |
| aio-plugin-tenant | `03b03a69c75cdf4a4eea3776a8d9a8a4a70ef827` |
| aio-plugin-file | `d7d5056c382c230ecf460996d7ec284cae0f14ac` |
| aio-plugin-dictionary | `35283ae2b1d78c6dd7ee23c5330106617d701f81` |

身份服务所有调用方固定到同一 Git 版本，并通过 Cargo 依赖树确认只有一个服务类型。以上提交均已推送并核对远端。

Linux 服务端 SHA-256：`6d59b17720b9f979f5ae7c18184763dbe4e2f672ae14f9f00ee8e87c2de62722`，本机产物与线上文件一致。
