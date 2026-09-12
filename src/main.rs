#![forbid(unsafe_code)]

mod plugins;
mod runtime;
#[cfg(feature = "server")]
mod server;
mod startup;

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
    use dioxus::prelude::*;

    rsx! {
        az_ui_components::UiStylesheets {}
        Workspace {}
    }
}

#[cfg(any(feature = "web", feature = "desktop"))]
#[dioxus::prelude::component]
fn Workspace() -> dioxus::prelude::Element {
    use az_dioxus_admin_shell::{
        ApplicationAccountItem, ApplicationMenuGroup, ApplicationRuntimePage, ApplicationUser,
        PluginApplication,
    };
    use dioxus::prelude::*;

    let mut last_application = use_signal(|| None::<startup::LoadedApplication>);
    let mut application = use_resource(move || {
        let previous = last_application.peek().clone();
        async move { startup::load(previous).await }
    });
    use_effect(move || {
        if let Some(Ok(value)) = application.read().as_ref() {
            last_application.set(Some(value.clone()));
        }
    });
    use_effect(move || {
        if application
            .read()
            .as_ref()
            .is_some_and(|result| result.as_ref().is_ok_and(|value| value.snapshot.is_none()))
        {
            spawn(async {
                let _ = document::eval("if (typeof caches !== 'undefined') { await Promise.all((await caches.keys()).filter(name => name.startsWith('aio-plugin-assets-v1-')).map(name => caches.delete(name))); } return true;").await;
            });
        }
    });
    use_future(move || async move {
        loop {
            let Ok(reason) = document::eval(include_str!("runtime/catalog_watch.js")).await else {
                break;
            };
            if application.finished() || reason.as_str() == Some("invalidated") {
                application.restart();
            }
        }
    });
    let result = application.read().as_ref().cloned();
    let result = match result {
        Some(Err(error)) => Some(
            last_application
                .read()
                .clone()
                .map(Ok)
                .unwrap_or(Err(error)),
        ),
        None => last_application.read().clone().map(Ok),
        result => result,
    };
    let Some(application_result) = result else {
        return rsx! { p { role: "status", aria_busy: "true", "正在加载工作区" } };
    };
    let snapshot = match application_result {
        Ok(startup::LoadedApplication {
            snapshot: Some(snapshot),
            ..
        }) => snapshot,
        Ok(startup::LoadedApplication { snapshot: None, .. }) => {
            return rsx! { aio_plugin_identity_client::LoginPage {} };
        }
        Err(error) => {
            return rsx! {
                p { role: "alert", "{error}" }
                az_ui_components::button::Button {
                    onclick: move |_| application.restart(),
                    "重试"
                }
            };
        }
    };
    let catalog = snapshot.catalog;
    let mut static_plugins = match plugins::client_catalog() {
        Ok(value) => value,
        Err(error) => {
            return rsx! { p { role: "alert", "加载应用页面失败: {error}" } };
        }
    };
    static_plugins.pages.retain(|page| {
        page.required_permission
            .is_none_or(|permission| snapshot.permissions.iter().any(|item| item == permission))
    });
    static_plugins.account_items.retain(|item| {
        item.required_permission
            .as_deref()
            .is_none_or(|permission| snapshot.permissions.iter().any(|value| value == permission))
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
                        snapshot
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
        .collect::<Vec<_>>();
    rsx! {
        for context in [catalog.session_context] {
          PluginApplication {
            key: "{context}",
            application_label: "AIO IDEA",
            pages: static_plugins.pages.clone(),
            account_items: account_items.clone(),
            runtime_pages: runtime_pages.clone(),
            runtime_page_versions: catalog.page_versions.clone(),
            workspace_id: catalog.tenant.id.clone(),
            workspace_context: catalog.context.clone(),
            render_runtime_page: runtime::client::render_page,
            on_account_action: runtime::client::account_action,
            user: ApplicationUser {
                label: catalog.user.label.clone(),
                handle: catalog.user.handle.clone(),
                initials: catalog.user.initials.clone(),
            },
          }
        }
    }
}
