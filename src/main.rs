#![forbid(unsafe_code)]

#[cfg(any(feature = "web", feature = "desktop"))]
mod pages;
mod plugins;
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
    server::run().await
}

#[cfg(any(feature = "web", feature = "desktop"))]
fn main() {
    dioxus::launch(App);
}

#[cfg(any(feature = "web", feature = "desktop"))]
#[allow(non_snake_case)]
fn App() -> dioxus::prelude::Element {
    use az_dioxus_admin_shell::{ApplicationUser, PluginApplication};
    use dioxus::prelude::*;

    let pages = match plugins::pages() {
        Ok(pages) => pages,
        Err(error) => return rsx! { p { "加载应用页面失败: {error}" } },
    };
    rsx! {
        PluginApplication {
            application_label: "AIO",
            pages,
            user: ApplicationUser {
                label: "用户".to_owned(),
                handle: String::new(),
                initials: "U".to_owned(),
            },
        }
    }
}
