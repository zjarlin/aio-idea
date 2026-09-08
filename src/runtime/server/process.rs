use std::{env, net::IpAddr, path::PathBuf, time::Duration};

use anyhow::{Context as _, Result, bail, ensure};
use az_plugin_manifest::{PageDefinition, validate_page_definitions};
use reqwest::{Method, StatusCode, Url, header};

use super::supervisor::{
    ProcessInstance, ReconcileProcessesRequest, StartProcessRequest, StopProcessRequest,
};

const MAX_PROCESS_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct ProcessManager {
    supervisor: reqwest::Client,
    runtime: reqwest::Client,
}

pub struct ProcessResponse {
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl ProcessManager {
    pub fn new() -> Result<Self> {
        let socket = env::var_os("AIO_PLUGIN_SUPERVISOR_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/aio-plugin-supervisor/supervisor.sock"));
        let supervisor = reqwest::Client::builder()
            .unix_socket(socket)
            .timeout(Duration::from_secs(90))
            .build()
            .context("创建插件监督器客户端失败")?;
        let runtime = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(30))
            .build()
            .context("创建 process 插件代理客户端失败")?;
        Ok(Self {
            supervisor,
            runtime,
        })
    }

    pub async fn health(&self) -> Result<()> {
        let response = self
            .supervisor
            .get("http://localhost/health")
            .send()
            .await
            .context("连接 process 插件监督器失败")?;
        ensure!(response.status().is_success(), "process 插件监督器未就绪");
        Ok(())
    }

    pub async fn start(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
    ) -> Result<ProcessInstance> {
        let response = self
            .supervisor
            .post("http://localhost/instances/start")
            .json(&StartProcessRequest {
                tenant_id: tenant_id.to_owned(),
                source_id: source_id.to_owned(),
                revision: revision.to_owned(),
            })
            .send()
            .await
            .context("请求启动 process 插件失败")?;
        decode_supervisor_response(response).await
    }

    pub async fn stop(&self, instance_id: &str) -> Result<()> {
        let response = self
            .supervisor
            .post("http://localhost/instances/stop")
            .json(&StopProcessRequest {
                instance_id: instance_id.to_owned(),
            })
            .send()
            .await
            .context("请求停止 process 插件失败")?;
        if response.status().is_success() {
            return Ok(());
        }
        bail!("停止 process 插件失败: {}", response_text(response).await);
    }

    pub(super) async fn reconcile(&self, instances: Vec<StartProcessRequest>) -> Result<()> {
        let response = self
            .supervisor
            .post("http://localhost/instances/reconcile")
            .json(&ReconcileProcessesRequest { instances })
            .send()
            .await
            .context("请求清理孤立 process 插件失败")?;
        if response.status().is_success() {
            return Ok(());
        }
        bail!(
            "清理孤立 process 插件失败: {}",
            response_text(response).await
        );
    }

    pub async fn load_pages(&self, endpoint: &str) -> Result<Vec<PageDefinition>> {
        let endpoint = process_url(endpoint, "/aio/definition", None)?;
        let response = self
            .runtime
            .get(endpoint)
            .send()
            .await
            .context("请求 process 插件页面定义失败")?;
        ensure!(
            response.status().is_success(),
            "process 插件页面定义返回失败状态"
        );
        ensure_response_size(&response)?;
        let pages = response
            .json::<Vec<PageDefinition>>()
            .await
            .context("解析 process 插件 PageDefinition 失败")?;
        validate_page_definitions(&pages)?;
        Ok(pages)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn request(
        &self,
        endpoint: &str,
        method: &str,
        path: &str,
        query: Option<&str>,
        body: Vec<u8>,
        content_type: Option<&str>,
        tenant_id: &str,
        user_id: &str,
    ) -> Result<ProcessResponse> {
        let method = Method::from_bytes(method.as_bytes()).context("process 代理请求方法无效")?;
        let url = process_url(endpoint, path, query)?;
        let mut request = self
            .runtime
            .request(method, url)
            .header("x-aio-tenant-id", tenant_id)
            .header("x-aio-user-id", user_id)
            .body(body);
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let response = request.send().await.context("process 插件请求失败")?;
        ensure_response_size(&response)?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .bytes()
            .await
            .context("读取 process 插件响应失败")?;
        ensure!(
            body.len() as u64 <= MAX_PROCESS_RESPONSE_BYTES,
            "process 插件响应超过 4 MiB 配额"
        );
        Ok(ProcessResponse {
            status,
            content_type,
            body: body.to_vec(),
        })
    }
}

async fn decode_supervisor_response(response: reqwest::Response) -> Result<ProcessInstance> {
    if response.status().is_success() {
        let instance = response
            .json::<ProcessInstance>()
            .await
            .context("解析 process 插件实例失败")?;
        validate_endpoint(&instance.endpoint)?;
        return Ok(instance);
    }
    bail!("启动 process 插件失败: {}", response_text(response).await);
}

async fn response_text(response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    format!(
        "HTTP {status}: {}",
        body.chars().take(4096).collect::<String>()
    )
}

fn ensure_response_size(response: &reqwest::Response) -> Result<()> {
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_PROCESS_RESPONSE_BYTES),
        "process 插件响应超过 4 MiB 配额"
    );
    Ok(())
}

fn process_url(endpoint: &str, path: &str, query: Option<&str>) -> Result<Url> {
    validate_endpoint(endpoint)?;
    ensure!(
        path.starts_with('/') && !path.starts_with("//"),
        "process 插件代理路径无效"
    );
    let mut url = Url::parse(endpoint).context("process 插件 endpoint 无效")?;
    url.set_path(path);
    url.set_query(query);
    Ok(url)
}

fn validate_endpoint(endpoint: &str) -> Result<()> {
    let url = Url::parse(endpoint).context("process 插件 endpoint 无效")?;
    ensure!(
        url.scheme() == "http"
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port() == Some(8080),
        "process 插件 endpoint 不符合内部代理边界"
    );
    let host = url.host_str().context("process 插件 endpoint 缺少 IP")?;
    let ip = host
        .parse::<IpAddr>()
        .context("process 插件 endpoint 必须是 IP")?;
    ensure!(
        match ip {
            IpAddr::V4(ip) => ip.is_private(),
            IpAddr::V6(ip) => ip.is_unique_local(),
        },
        "process 插件 endpoint 必须是私有网络 IP"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_private_fixed_port_endpoints() -> Result<()> {
        validate_endpoint("http://172.29.0.2:8080")?;
        assert!(validate_endpoint("https://172.29.0.2:8080").is_err());
        assert!(validate_endpoint("http://127.0.0.1:8080").is_err());
        assert!(validate_endpoint("http://172.29.0.2:80").is_err());
        assert!(validate_endpoint("http://example.com:8080").is_err());
        Ok(())
    }

    #[test]
    fn preserves_only_explicit_proxy_path_and_query() -> Result<()> {
        let url = process_url("http://172.29.0.2:8080", "/echo", Some("value=1"))?;
        assert_eq!(url.as_str(), "http://172.29.0.2:8080/echo?value=1");
        assert!(process_url("http://172.29.0.2:8080", "//other", None).is_err());
        Ok(())
    }
}
