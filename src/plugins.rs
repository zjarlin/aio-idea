// 此文件由 aio plugin sync 生成。
#[cfg(any(feature = "web", feature = "desktop"))]
pub struct ClientCatalog {
    pub pages: Vec<az_dioxus_admin_shell::ApplicationPage>,
    pub account_items: Vec<az_dioxus_admin_shell::ApplicationAccountItem>,
}

#[cfg(any(feature = "web", feature = "desktop"))]
pub fn client_catalog() -> anyhow::Result<ClientCatalog> {
    use anyhow::Context as _;
    use dill::{Catalog, CatalogBuilder};

    let mut builder = CatalogBuilder::new();
    crate::pages::home::register(&mut builder);
    aio_plugin_identity_client::register(&mut builder);
    aio_plugin_marketplace_client::register(&mut builder);
    aio_plugin_rbac_client::register(&mut builder);
    aio_plugin_settings::register(&mut builder);
    aio_plugin_account::register(&mut builder);
    aio_plugin_tenant_client::register(&mut builder);
    builder.validate().context("校验页面插件依赖图失败")?;
    let catalog: Catalog = builder.build();
    Ok(ClientCatalog {
        pages: az_dioxus_admin_shell::collect_application_pages(&catalog)?,
        account_items: az_dioxus_admin_shell::collect_application_account_items(&catalog)?,
    })
}

#[cfg(feature = "server")]
pub fn server_catalog() -> anyhow::Result<dill::Catalog> {
    use anyhow::Context as _;
    use dill::{Catalog, CatalogBuilder};

    let mut builder = CatalogBuilder::new();
    aio_plugin_identity_server::register(&mut builder)?;
    aio_plugin_marketplace_server::register(&mut builder);
    aio_plugin_rbac_server::register(&mut builder)?;
    aio_plugin_tenant_server::register(&mut builder)?;
    builder.validate().context("校验服务端插件依赖图失败")?;
    let catalog: Catalog = builder.build();
    Ok(catalog)
}

#[cfg(feature = "server")]
pub fn server_router(catalog: &dill::Catalog) -> anyhow::Result<axum::Router> {
    use anyhow::Context as _;

    let mut router = axum::Router::new();
    router = router.merge(aio_plugin_identity_server::router(catalog).context("装配身份插件失败")?);
    router =
        router.merge(aio_plugin_marketplace_server::router(catalog).context("装配服务端插件失败")?);
    router = router.merge(aio_plugin_rbac_server::router(catalog).context("装配权限插件失败")?);
    router = router.merge(aio_plugin_tenant_server::router(catalog).context("装配租户插件失败")?);
    Ok(router)
}
