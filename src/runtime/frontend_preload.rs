use dioxus::prelude::*;

#[component]
pub(crate) fn FrontendPreload(config: String) -> Element {
    let mut bridge = use_signal(|| None::<document::Eval>);
    use_future(move || {
        let config = config.clone();
        async move {
            let mut evaluator = document::eval(concat!(
                include_str!("frontend_cache.js"), "\n",
                include_str!("frontend_preload.js"), "\n",
                "const config = JSON.parse(await dioxus.recv());
                 const controller = new AbortController();
                 const leave = () => controller.abort();
                 window.addEventListener('pagehide', leave);
                 const timer = setTimeout(() => { void warmFrontendAssets(config, controller.signal).catch(() => {}); }, 500);
                 try { await dioxus.recv(); } finally {
                   clearTimeout(timer); controller.abort();
                   window.removeEventListener('pagehide', leave);
                 }"
            ));
            if evaluator.send(config).is_ok() {
                bridge.set(Some(evaluator));
                let _ = evaluator.recv::<serde_json::Value>().await;
            }
        }
    });
    use_drop(move || {
        if let Some(evaluator) = bridge() {
            let _ = evaluator.send(serde_json::json!({ "dispose": true }));
        }
    });
    rsx! {}
}
