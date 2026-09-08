# 服务端插件生命周期

`mod.rs` 只装配状态；`repository.rs` 负责 Git 发现与版本缓存，`store.rs` 负责 PostgreSQL 事务，`routes.rs` 负责 HTTP 边界。
