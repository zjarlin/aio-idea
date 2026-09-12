# Component 安装与发布

原生 `aio:plugin@2.0.0` 整包进入独立的持久执行槽。发布校验只更新市场版本，租户安装由用户显式选择；父子依赖按租户校验，不自动安装子插件。页面转换仅用于共享壳导航，业务请求直接使用 v2 二进制契约。

`AIO_COMPONENT_DATABASE_URL` 指向具有创建隔离角色权限的专用 PostgreSQL，数据库必须撤销 PUBLIC 权限。`AIO_COMPONENT_HOME` 是发布目录之外的持久目录，保存 0600 的 `keyring.json` 和 `objects/`；未配置数据库时不开启原生 Component 安装。

安装记录是版本激活的提交点。安装失败恢复上一执行槽，重启时按安装记录恢复执行槽；停用和卸载保留业务 schema、对象及版本历史。声明的业务权限只授予同租户已持有 `plugin:manage` 的角色。
