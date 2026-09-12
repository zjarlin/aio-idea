use aio_plugin_identity_server::SessionContext;
use anyhow::{Result, bail};
use async_trait::async_trait;
use az_plugin_contract::{InvocationScope, RequestContext};
use az_plugin_runtime::{
    HostServices,
    bindings::aio::plugin::transport::{Request, Response},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(super) struct Services {
    requests: Mutex<HashMap<String, (RequestContext, Vec<String>)>>,
}
pub(super) struct Authorization {
    pub context: RequestContext,
    services: Arc<Services>,
}
impl Services {
    pub fn enter(self: &Arc<Self>, session: &SessionContext) -> Result<Authorization> {
        let context = RequestContext {
            tenant_id: Some(session.tenant_id.clone()),
            user_id: Some(session.user_id.clone()),
            session_id: Some(session.session_id.clone()),
            request_id: uuid::Uuid::new_v4().to_string(),
        };
        self.requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?
            .insert(
                context.request_id.clone(),
                (context.clone(), session.permissions.clone()),
            );
        Ok(Authorization {
            context,
            services: self.clone(),
        })
    }
}
impl Drop for Authorization {
    fn drop(&mut self) {
        if let Ok(mut requests) = self.services.requests.lock() {
            requests.remove(&self.context.request_id);
        }
    }
}
#[async_trait]
impl HostServices for Services {
    async fn authorize(&self, scope: &InvocationScope, permission: &str) -> Result<bool> {
        let requests = self
            .requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?;
        Ok(requests
            .get(&scope.context.request_id)
            .is_some_and(|(context, permissions)| {
                context == &scope.context && permissions.iter().any(|p| p == permission || p == "*")
            }))
    }
    async fn manage(&self, _: &InvocationScope, _: Request) -> Result<Response> {
        bail!("当前执行器未授予管理接口")
    }
}
