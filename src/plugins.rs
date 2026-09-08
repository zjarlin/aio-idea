// 此文件由 aio plugin sync 生成。
#[cfg(any(feature = "web", feature = "desktop"))]
pub fn pages() -> anyhow::Result<Vec<az_dioxus_admin_shell::ApplicationPage>> {
    use anyhow::Context as _;
    use dill::{Catalog, CatalogBuilder};

    let mut builder = CatalogBuilder::new();
    crate::pages::home::register(&mut builder);
    aio_plugin_hello_client::register(&mut builder);
    builder.validate().context("校验页面插件依赖图失败")?;
    let catalog: Catalog = builder.build();
    az_dioxus_admin_shell::collect_application_pages(&catalog)
}

#[cfg(feature = "server")]
pub fn server_router() -> anyhow::Result<axum::Router> {
    use anyhow::Context as _;
    use dill::{Catalog, CatalogBuilder};

    let mut builder = CatalogBuilder::new();
    aio_plugin_hello_server::register(&mut builder);
    builder.validate().context("校验服务端插件依赖图失败")?;
    let catalog: Catalog = builder.build();
    let mut router = axum::Router::new();
    router = router.merge(aio_plugin_hello_server::router(&catalog).context("装配服务端插件失败")?);
    Ok(router)
}
