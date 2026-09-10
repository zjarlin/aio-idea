use az_dioxus_admin_shell::{ApplicationPage, ApplicationPlugin, ApplicationScene};
use dill::CatalogBuilder;
use dioxus::prelude::*;

#[derive(Debug)]
pub struct HomePlugin;

impl ApplicationPlugin for HomePlugin {
    fn pages(&self) -> Vec<ApplicationPage> {
        vec![ApplicationPage {
            id: "home",
            label: "首页",
            icon: Some("house"),
            scene: ApplicationScene {
                id: "workspace",
                label: "工作区",
            },
            menu_path: Vec::new(),
            required_permission: None,
            render: HomePage,
        }]
    }
}

pub fn register(builder: &mut CatalogBuilder) {
    builder
        .add_value(HomePlugin)
        .bind::<dyn ApplicationPlugin, HomePlugin>();
}

#[allow(non_snake_case)]
fn HomePage() -> Element {
    rsx! {
        section {
            h2 { "首页" }
            p { "从这里开始集成页面。" }
        }
    }
}
