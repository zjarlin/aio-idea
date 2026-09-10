#![forbid(unsafe_code)]

#[cfg(any(feature = "web", feature = "desktop"))]
mod pages;
mod plugins;
mod runtime;
#[cfg(feature = "server")]
mod server;

#[cfg(all(feature = "server", any(feature = "web", feature = "desktop")))]
compile_error!("server 不能和 web 或 desktop 同时启用");

#[cfg(not(any(feature = "web", feature = "desktop", feature = "server")))]
fn main() {
    eprintln!("请选择 --features web、desktop 或 server");
}

#[cfg(feature = "server")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("supervisor") {
        return runtime::server::run_supervisor().await;
    }
    server::run().await
}

#[cfg(any(feature = "web", feature = "desktop"))]
fn main() {
    dioxus::launch(App);
}

#[cfg(any(feature = "web", feature = "desktop"))]
#[allow(non_snake_case)]
fn App() -> dioxus::prelude::Element {
    use az_dioxus_admin_shell::{
        ApplicationAccountItem, ApplicationMenuGroup, ApplicationRuntimePage, ApplicationUser,
        PluginApplication,
    };
    use dioxus::prelude::*;

    let application = use_resource(|| async {
        let session = aio_plugin_identity_client::load_session().await?;
        let catalog = match session.as_ref() {
            Some(_) => Some(runtime::client::catalog().await?),
            None => None,
        };
        Ok::<_, String>((session, catalog))
    });
    let Some(application_result) = application.read().as_ref().cloned() else {
        return standalone_page(rsx! { p { "正在验证会话" } });
    };
    let (session, catalog) = match application_result {
        Ok((Some(session), Some(catalog))) => (session, catalog),
        Ok((None, _)) => {
            return standalone_page(rsx! { aio_plugin_identity_client::LoginPage {} });
        }
        Ok((Some(_), None)) => {
            return standalone_page(rsx! { p { role: "alert", "插件目录没有返回数据" } });
        }
        Err(error) => {
            return standalone_page(rsx! { p { role: "alert", "验证会话失败: {error}" } });
        }
    };
    let mut static_plugins = match plugins::client_catalog() {
        Ok(value) => value,
        Err(error) => {
            return standalone_page(rsx! { p { role: "alert", "加载应用页面失败: {error}" } });
        }
    };
    static_plugins.pages.retain(|page| {
        page.required_permission
            .is_none_or(|permission| session.permissions.iter().any(|item| item == permission))
    });
    static_plugins.account_items.retain(|item| {
        item.required_permission
            .as_deref()
            .is_none_or(|permission| session.permissions.iter().any(|value| value == permission))
    });
    let mut account_items = static_plugins.account_items;
    account_items.extend(
        catalog
            .account_items
            .into_iter()
            .filter(|item| {
                item.required_permission
                    .as_deref()
                    .is_none_or(|permission| {
                        session
                            .permissions
                            .iter()
                            .any(|candidate| candidate == permission)
                    })
            })
            .map(|item| ApplicationAccountItem {
                id: item.id,
                label: item.label,
                icon: item.icon,
                page_id: Some(item.page_id),
                required_permission: item.required_permission,
                destructive: false,
            }),
    );
    let runtime_pages = catalog
        .pages
        .into_iter()
        .map(|page| ApplicationRuntimePage {
            id: page.id,
            label: page.label,
            icon: page.icon,
            scene_id: page.scene.id,
            scene_label: page.scene.label,
            menu_path: page
                .menu_path
                .into_iter()
                .map(|group| ApplicationMenuGroup {
                    id: group.id,
                    label: group.label,
                    icon: group.icon,
                })
                .collect(),
            required_permission: page.required_permission,
            definition: serde_json::to_string(&page.body).unwrap_or_default(),
        })
        .collect();
    rsx! {
        PluginApplication {
            application_label: "AIO IDEA",
            pages: static_plugins.pages,
            account_items,
            runtime_pages,
            render_runtime_page: runtime::client::render_page,
            on_account_action: runtime::client::account_action,
            user: ApplicationUser {
                label: catalog.user.label,
                handle: catalog.user.handle,
                initials: catalog.user.initials,
            },
        }
    }
}

#[cfg(any(feature = "web", feature = "desktop"))]
fn standalone_page(content: dioxus::prelude::Element) -> dioxus::prelude::Element {
    use dioxus::prelude::*;

    rsx! {
        az_ui_components::UiStylesheets {}
        {content}
    }
}
