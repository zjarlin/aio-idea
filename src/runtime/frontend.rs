use dioxus::prelude::*;
use serde::Deserialize;

use super::RuntimeResponse;

#[derive(Clone, Deserialize, PartialEq)]
struct FrontendMount {
    token: String,
    src: String,
    revision: String,
}

#[component]
pub(super) fn RuntimeFrontend(page_id: String, label: String) -> Element {
    let mount = use_resource(move || {
        let page_id = page_id.clone();
        async move {
            let response = gloo_net::http::Request::post("/api/runtime/frontend/mount")
                .json(&serde_json::json!({ "page_id": page_id }))
                .map_err(|error| error.to_string())?
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.ok() {
                return Err(response
                    .text()
                    .await
                    .unwrap_or_else(|_| "挂载前端失败".to_owned()));
            }
            response
                .json::<RuntimeResponse<FrontendMount>>()
                .await
                .map(|response| response.data)
                .map_err(|error| error.to_string())
        }
    });
    match mount.read().as_ref() {
        Some(Ok(mount)) => {
            rsx! { MountedFrontend { key: "{mount.token}", mount: mount.clone(), label } }
        }
        Some(Err(error)) => rsx! { p { role: "alert", "加载插件页面失败: {error}" } },
        None => rsx! { p { role: "status", "正在加载插件页面" } },
    }
}

#[component]
fn MountedFrontend(mount: FrontendMount, label: String) -> Element {
    let mut bridge = use_signal(|| None::<document::Eval>);
    let mut error = use_signal(|| None::<String>);
    let frame_id = format!("aio-frontend-{}", mount.token);
    let config = serde_json::json!({ "id": frame_id, "token": mount.token, "src": mount.src });
    use_drop(move || {
        if let Some(bridge) = bridge() {
            let _ = bridge.send(serde_json::json!({ "dispose": true }));
        }
    });
    rsx! {
        if let Some(message) = error() {
            p { role: "alert", "{message}" }
        }
        iframe {
            id: frame_id,
            title: label,
            class: "w-full border-0",
            height: "640",
            "sandbox": "allow-scripts",
            referrerpolicy: "no-referrer",
            onmounted: move |_| {
                if bridge().is_none() {
                    let evaluator = document::eval(include_str!("frontend_host.js"));
                    match evaluator.send(config.clone()) {
                        Ok(()) => bridge.set(Some(evaluator)),
                        Err(cause) => error.set(Some(format!("启动插件通信桥失败: {cause}"))),
                    }
                }
            },
        }
    }
}
