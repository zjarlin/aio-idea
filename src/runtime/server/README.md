# 服务端插件生命周期

`mod.rs` 只装配状态；`repository.rs` 负责 Git 发现与版本缓存，`store.rs` 负责 PostgreSQL 生命周期事务，`marketplace_store.rs` 负责市场索引缓存，`routes.rs` 负责 HTTP 边界。

市场请求只从 PostgreSQL 缓存读取。远程 HTTPS/Git registry 在启动和请求后异步刷新；刷新失败只保留最近成功索引并写入同步状态，不影响市场页面、已安装插件或租户组合。

`POST /api/runtime/plugins/publish` 接收 CI 提交的锁定 SHA、清单、artifact 和 SHA-256，支持 `page-definition`、`wasm-component` 和 `process`，并复用安装事务的校验、健康检查、原子激活和回滚。发布凭证由 `plugin_publish_credentials` 按租户和 Git 来源限制，可通过运行时 API 轮换或撤销；不使用全局 CI 发布令牌。

宿主启动时从 PostgreSQL 的活动租户组合生成 process 实例白名单，并调用监督器清理不在白名单内的受管容器和隔离网络；数据库中的孤立 active 记录同时转为 stopped。

页面动作统一进入 `POST /api/runtime/pages/action`。Wasm Component 复用 `handle`；process 插件通过仅宿主可访问的 `POST /aio/action` 接收同一事件。该内部路径不属于插件公开 `routes`，不能由浏览器经服务代理直接调用。
