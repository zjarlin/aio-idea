use anyhow::Result;

use super::{
    RuntimeState,
    lifecycle::{
        cleanup_new_process, cleanup_new_wasm, deactivate_previous_wasm, prepare_process,
        prepare_wasm, restore_process_binding, stop_previous_process,
    },
    repository::DiscoveredPlugin,
};
use crate::runtime::PluginRuntime;

pub(super) struct ActivatedPlugin {
    pub source_id: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub page_count: usize,
}

pub(super) async fn activate(
    state: &RuntimeState,
    tenant_id: &str,
    mut discovered: DiscoveredPlugin,
    validation_detail: &str,
) -> Result<ActivatedPlugin> {
    if let Some(source_id) = state.store.source_id(&discovered.git).await? {
        discovered.source_id = source_id;
    }
    let current_revision = discovered.revision.clone();
    state
        .store
        .record_lifecycle_event(
            tenant_id,
            &discovered.source_id,
            None,
            "validate",
            validation_detail,
        )
        .await?;
    let previous = state
        .store
        .active_process(tenant_id, &discovered.source_id)
        .await?;
    let previous_target = if previous.is_some() {
        Some(
            state
                .store
                .bound_runtime(tenant_id, &discovered.source_id)
                .await?,
        )
    } else {
        None
    };
    let previous_wasm = state
        .store
        .active_wasm_revision(tenant_id, &discovered.source_id)
        .await?;
    let instance = if discovered.runtime == PluginRuntime::Process {
        Some(prepare_process(state, tenant_id, &mut discovered).await?)
    } else {
        None
    };
    let wasm = if discovered.runtime == PluginRuntime::WasmComponent {
        Some(prepare_wasm(state, tenant_id, &mut discovered).await?)
    } else {
        None
    };
    if instance.is_some() || wasm.is_some() {
        state
            .store
            .record_lifecycle_event(
                tenant_id,
                &discovered.source_id,
                None,
                "health-check",
                "运行时健康检查通过",
            )
            .await?;
    }
    let activated = ActivatedPlugin {
        source_id: discovered.source_id.clone(),
        revision: discovered.revision.clone(),
        runtime: discovered.runtime,
        page_count: discovered.pages.len(),
    };
    if let Err(error) = stop_previous_process(state, previous, instance.as_ref()).await {
        let _ = cleanup_new_process(state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            state,
            tenant_id,
            &activated.source_id,
            &current_revision,
            wasm.as_ref(),
        );
        return Err(error);
    }
    if let Err(error) = state
        .store
        .activate(tenant_id, discovered, instance.as_ref())
        .await
    {
        let _ = cleanup_new_process(state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            state,
            tenant_id,
            &activated.source_id,
            &current_revision,
            wasm.as_ref(),
        );
        if let Err(recovery) = restore_process_binding(
            state,
            tenant_id,
            &activated.source_id,
            previous_target.as_ref(),
        )
        .await
        {
            return Err(anyhow::anyhow!(
                "激活插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            ));
        }
        return Err(error);
    }
    deactivate_previous_wasm(
        state,
        tenant_id,
        &activated.source_id,
        previous_wasm.as_deref(),
        wasm.as_ref().map(|_| current_revision.as_str()),
    )?;
    Ok(activated)
}
