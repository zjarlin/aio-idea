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
    aio_plugin_identity_client::register(&mut builder);
    aio_plugin_marketplace_client::register(&mut builder);
    aio_plugin_rbac_client::register(&mut builder);
    aio_plugin_dictionary_client::register(&mut builder);
    aio_plugin_file_client::register(&mut builder);
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
    aio_plugin_dictionary_server::register(&mut builder)?;
    aio_plugin_file_server::register(&mut builder)?;
    aio_plugin_identity_server::register(&mut builder)?;
    aio_plugin_marketplace_server::register(&mut builder)?;
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
    router =
        router.merge(aio_plugin_dictionary_server::router(catalog).context("装配字典插件失败")?);
    router = router.merge(aio_plugin_file_server::router(catalog).context("装配文件插件失败")?);
    router = router.merge(aio_plugin_identity_server::router(catalog).context("装配身份插件失败")?);
    router =
        router.merge(aio_plugin_marketplace_server::router(catalog).context("装配服务端插件失败")?);
    router = router.merge(aio_plugin_rbac_server::router(catalog).context("装配权限插件失败")?);
    router = router.merge(aio_plugin_tenant_server::router(catalog).context("装配租户插件失败")?);
    Ok(router)
}

#[cfg(all(test, any(feature = "web", feature = "desktop")))]
mod tests {
    use std::collections::HashSet;

    use super::client_catalog;

    #[test]
    fn system_plugins_contribute_navigation_and_account_surface() -> anyhow::Result<()> {
        let catalog = client_catalog()?;
        let page_ids = catalog
            .pages
            .iter()
            .map(|page| page.id)
            .collect::<HashSet<_>>();
        let account_ids = catalog
            .account_items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<HashSet<_>>();

        for page_id in [
            "users",
            "roles",
            "dictionary-management",
            "file-list",
            "profile",
            "settings",
            "marketplace",
            "tenants",
        ] {
            assert!(page_ids.contains(page_id), "缺少系统页面: {page_id}");
        }
        assert!(!page_ids.contains("home"), "壳不能贡献内置首页");

        let users = catalog
            .pages
            .iter()
            .find(|page| page.id == "users")
            .expect("缺少用户管理页面");
        assert_eq!(users.scene.id, "system");
        assert_eq!(
            users
                .menu_path
                .iter()
                .map(|group| group.id.as_str())
                .collect::<Vec<_>>(),
            ["system-management"]
        );

        let files = catalog
            .pages
            .iter()
            .find(|page| page.id == "file-list")
            .expect("缺少文件管理页面");
        assert_eq!(files.scene.id, "system");
        assert_eq!(
            files
                .menu_path
                .iter()
                .map(|group| group.id.as_str())
                .collect::<Vec<_>>(),
            ["infrastructure", "file-management"]
        );
        assert_eq!(files.required_permission, Some("file:manage"));
        for account_id in [
            "profile",
            "settings",
            "marketplace",
            "tenant-switcher",
            "logout",
        ] {
            assert!(
                account_ids.contains(account_id),
                "缺少账户入口: {account_id}"
            );
        }
        Ok(())
    }
}
