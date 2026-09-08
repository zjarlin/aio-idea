use az_dioxus_admin_shell::ApplicationRuntimePage;
use az_ui_components::button::{Button, ButtonVariant};
use dioxus::prelude::*;

use super::{PageBody, RuntimeCatalog, RuntimeResponse};

pub async fn catalog() -> Result<RuntimeCatalog, String> {
    let response = gloo_net::http::Request::get("/api/runtime/catalog")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    response
        .json::<RuntimeResponse<RuntimeCatalog>>()
        .await
        .map(|response| response.data)
        .map_err(|error| error.to_string())
}

pub fn render_page(page: ApplicationRuntimePage) -> Element {
    match serde_json::from_str::<PageBody>(&page.definition) {
        Ok(PageBody::Counter { title, button }) => rsx! {
            RuntimeCounter { title, button }
        },
        Ok(PageBody::Text { title, content }) => rsx! {
            section { h2 { "{title}" } p { "{content}" } }
        },
        Err(error) => rsx! { p { role: "alert", "页面定义无效: {error}" } },
    }
}

pub fn account_action(action: String) {
    if action == "logout" {
        spawn(async move {
            let _ = gloo_net::http::Request::post("/api/auth/logout")
                .send()
                .await;
            reload();
        });
    }
}

#[component]
fn RuntimeCounter(title: String, button: String) -> Element {
    let mut count = use_signal(|| 0_u64);
    rsx! {
        section {
            h2 { "{title}" }
            p { "计数：{count}" }
            Button {
                r#type: "button",
                variant: ButtonVariant::Outline,
                aria_label: button.clone(),
                onclick: move |_| count.with_mut(|value| *value = value.saturating_add(1)),
                "{button}"
            }
        }
    }
}

pub fn reload() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}
