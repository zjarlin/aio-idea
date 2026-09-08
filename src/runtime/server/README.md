# 服务端插件生命周期

`mod.rs` 只装配状态；`repository.rs` 负责 Git 发现与版本缓存，`store.rs` 负责 PostgreSQL 事务，`routes.rs` 负责 HTTP 边界。

宿主启动时从 PostgreSQL 的活动租户组合生成 process 实例白名单，并调用监督器清理不在白名单内的受管容器和隔离网络；数据库中的孤立 active 记录同时转为 stopped。

页面动作统一进入 `POST /api/runtime/pages/action`。Wasm Component 复用 `handle`；process 插件通过仅宿主可访问的 `POST /aio/action` 接收同一事件。该内部路径不属于插件公开 `routes`，不能由浏览器经服务代理直接调用。
