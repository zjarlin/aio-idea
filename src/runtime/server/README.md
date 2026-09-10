# 服务端插件生命周期

`mod.rs` 只装配状态；`repository.rs` 负责 Git 发现与版本缓存，`store.rs` 负责 PostgreSQL 生命周期事务，`marketplace_store.rs` 负责市场索引缓存，`routes.rs` 负责 HTTP 边界。

市场请求只从 PostgreSQL 缓存读取。远程 HTTPS/Git registry 在启动和请求后异步刷新；刷新失败只保留最近成功索引并写入同步状态，不影响市场页面、已安装插件或租户组合。

`POST /api/runtime/plugins/publish` 接收 CI 提交的锁定 SHA、清单、artifact、SHA-256，以及包含原始 commit 和路径所需 tree 对象的 `git_proof`。宿主离线校验 Git 对象哈希，并确认清单与 artifact 确实属于该 commit，无需在发布请求中回连远程 Git；随后复用安装事务的协议校验、健康检查、原子激活和回滚。只有激活成功的版本才更新市场缓存。发布凭证由 `plugin_publish_credentials` 按租户和 Git 来源限制，只有 `AIO_PLUGIN_PUBLISH_ACCOUNTS` 明确授权的平台账号可以通过运行时 API 创建或撤销；不使用全局 CI 发布令牌。

宿主启动时从 PostgreSQL 的活动租户组合生成 process 实例白名单，并调用监督器清理不在白名单内的受管容器和隔离网络；数据库中的孤立 active 记录同时转为 stopped。

远程 Git 和 HTTPS registry 仅允许标准 443 端口的公网目标，解析出的地址会固定到实际连接并禁用重定向。单次仓库展开、条目数、市场索引响应以及整个版本缓存分别受配额限制；252 默认最多保留 1 GiB、10 万个文件系统条目和 512 个版本，可通过对应的 `AIO_PLUGIN_CACHE_MAX_*` 环境变量收紧。

页面动作统一进入 `POST /api/runtime/pages/action`。Wasm Component 复用 `handle`；process 插件通过仅宿主可访问的 `POST /aio/action` 接收同一事件。该内部路径不属于插件公开 `routes`，不能由浏览器经服务代理直接调用。
