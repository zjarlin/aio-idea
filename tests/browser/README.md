# 浏览器验收

此目录在真实宿主上验证登录、导航、插件安装、iframe、租户上下文与资源加载。使用已安装的 Playwright 和 Chrome；生产会话通过私有 `AIO_COOKIE_FILE` 传入，不提交凭据。

`component-marketplace.cjs` 上传大屏原生整包，通过市场 UI 安装，导入 CSV、拖拽绑定、保存发布、刷新和手机播放，再验证坏包拒绝并卸载。图表检查读取 canvas 像素，报告保存在 `target/component-delivery/`。`component-preview.cjs` 启动隔离数据库副本，只绑定回环地址。副本的准备与正式发布见 `deploy/252/README.md`。
