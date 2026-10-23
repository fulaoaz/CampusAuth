use super::models::{ConnectionConfig, Service, Status};
use super::storage::StoredAccount;
use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use reqwest::blocking::{Client, Response};
use reqwest::cookie::Jar;
use reqwest::header::{CONTENT_TYPE, LOCATION};
use reqwest::redirect::Policy;
use serde_json::Value;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ",
    "(KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 CampusAuth/",
    env!("CARGO_PKG_VERSION")
);

pub fn quote_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(byte) {
            out.push(*byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 15) as usize] as char);
        }
    }
    out
}

pub fn login_body(username: &str, password: &str, service: &str, query: &str) -> Result<String> {
    if !has_login_parameters(query) {
        bail!("认证参数缺少 wlanuserip，未提交登录。")
    }
    Ok(format!(
        "userId={}&password={}&service={}&queryString={}&operatorPwd=&operatorUserId=&validcode=&passwordEncrypt=false",
        quote_component(username),
        quote_component(password),
        quote_component(&quote_component(service)),
        quote_component(&quote_component(query))
    ))
}

pub fn has_login_parameters(query: &str) -> bool {
    url::form_urlencoded::parse(query.as_bytes())
        .any(|(key, value)| key == "wlanuserip" && !value.trim().is_empty())
}

pub(crate) fn gateway_url(cfg: &ConnectionConfig) -> Result<Url> {
    let raw = cfg.gateway.trim();
    if raw.is_empty() {
        bail!("请在设置中填写学校的 ePortal 认证网关。")
    }
    let value = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("http://{raw}")
    };
    let mut url = Url::parse(&value).context("认证网关地址格式不正确")?;
    validate_url(&url)?;
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

pub(crate) fn validate_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("仅支持不包含用户名和密码的 HTTP/HTTPS 地址。")
    }
    Ok(())
}

pub(crate) fn validate_config(cfg: &ConnectionConfig) -> Result<()> {
    if cfg.gateway.len() > 2048
        || cfg.service.len() > 1024
        || cfg.user_group.len() > 1024
        || cfg.profile_id.len() > 128
        || cfg.display_name.len() > 256
    {
        bail!("学校认证设置内容过长，请检查输入。")
    }
    gateway_url(cfg)?;
    if cfg.password_encrypt {
        bail!("此版本暂不支持要求 RSA 密码加密的 ePortal 网关，请联系开发者提供协议适配。")
    }
    if !(1..=30).contains(&cfg.timeout_seconds)
        || !(1..=6).contains(&cfg.retry_per_account)
        || cfg.retry_interval_seconds > 30
    {
        bail!("网络超时需为 1–30 秒，每账号重试次数需为 1–6 次，重试间隔不能超过 30 秒。")
    }
    if cfg.probe_urls.is_empty() || cfg.probe_urls.len() > 8 || cfg.discovery_urls.len() > 8 {
        bail!("请配置 1–8 个联网检测地址，认证发现地址最多 8 个。")
    }
    for value in cfg.probe_urls.iter().chain(cfg.discovery_urls.iter()) {
        if value.len() > 2048 {
            bail!("网络检测或认证发现地址过长。")
        }
        validate_url(&Url::parse(value).context("网络检测或认证发现地址格式不正确")?)?;
    }
    Ok(())
}

pub(crate) struct PortalSession {
    config: ConnectionConfig,
    client: Client,
    jar: Arc<Jar>,
    base: Url,
    root: Url,
}

impl PortalSession {
    pub fn new(cfg: &ConnectionConfig) -> Result<Self> {
        let root = gateway_url(cfg)?;
        let jar = Arc::new(Jar::default());
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .cookie_provider(jar.clone())
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(cfg.timeout_seconds.clamp(1, 30)))
            .build()
            .context("无法建立网络客户端")?;
        let base = root.join("eportal/")?;
        Ok(Self {
            config: cfg.clone(),
            client,
            jar,
            base,
            root,
        })
    }

    fn remaining(deadline: Instant, requested: Duration) -> Result<Duration> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining < Duration::from_millis(10) {
            bail!("网络操作已超时，请稍后重试。")
        }
        Ok(remaining.min(requested))
    }

    fn get(&self, url: Url, deadline: Instant, max_timeout: Duration) -> Result<Response> {
        validate_url(&url)?;
        self.client
            .get(url)
            .timeout(Self::remaining(deadline, max_timeout)?)
            .send()
            .map_err(|error| anyhow!(network_error(&error)))
    }

    fn body(response: Response) -> Result<String> {
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .context("读取校园网响应失败")?;
        if bytes.len() as u64 > MAX_RESPONSE {
            bail!("校园网响应过大，已停止处理。")
        }
        if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            return Ok(String::from_utf8_lossy(&bytes[3..]).into_owned());
        }
        if content_type.contains("charset=gbk")
            || content_type.contains("charset=gb2312")
            || content_type.contains("charset=gb18030")
            || content_type.contains("charset=\"gbk\"")
            || content_type.contains("charset=\"gb18030\"")
        {
            return Ok(encoding_rs::GB18030.decode(&bytes).0.into_owned());
        }
        match String::from_utf8(bytes) {
            Ok(text) => Ok(text),
            Err(error) => Ok(encoding_rs::GB18030.decode(error.as_bytes()).0.into_owned()),
        }
    }

    pub fn probe(&self, deadline: Instant, brief: bool) -> (bool, String) {
        let (online, reason) = self.probe_state(deadline, brief);
        (online == Some(true), reason)
    }

    pub fn probe_state(&self, deadline: Instant, brief: bool) -> (Option<bool>, String) {
        let limit = if brief {
            2
        } else {
            self.config.probe_urls.len()
        };
        let timeout = Duration::from_secs(if brief {
            2
        } else {
            self.config.timeout_seconds
        });
        let mut reason = "所有联网检测地址均不可达，请检查网卡和校园网连接。".to_owned();
        let mut received_response = false;
        for raw in self.config.probe_urls.iter().take(limit) {
            let Ok(url) = Url::parse(raw) else {
                continue;
            };
            let response = match self.get(url.clone(), deadline, timeout) {
                Ok(response) => response,
                Err(error) => {
                    reason = error.to_string();
                    continue;
                }
            };
            received_response = true;
            let code = response.status();
            if code.is_redirection() {
                if let Some(location) = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                {
                    if portal_text(location) {
                        return (Some(false), "校园网要求登录认证。".into());
                    }
                }
                continue;
            }
            if code.as_u16() == 204 {
                return (Some(true), "互联网连接正常。".into());
            }
            if code.is_success() {
                if let Ok(body) = Self::body(response) {
                    if portal_text(&body) {
                        return (Some(false), "当前返回校园网认证页面，请登录。".into());
                    }
                    if known_probe_matches(&url, &body) {
                        return (Some(true), "互联网检测通过。".into());
                    }
                    reason = "联网检测响应不符合预期，可能仍被校园网认证页面拦截。".into();
                }
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        (received_response.then_some(false), reason)
    }

    pub fn online_info(&self, deadline: Instant, brief: bool) -> Result<Value> {
        let url = self.base.join("InterFace.do?method=getOnlineUserInfo")?;
        let response = self.get(
            url,
            deadline,
            Duration::from_secs(if brief {
                2
            } else {
                self.config.timeout_seconds
            }),
        )?;
        if !response.status().is_success() {
            bail!("认证网关暂时无法查询在线状态。")
        }
        serde_json::from_str(&Self::body(response)?).context("认证网关未返回有效在线信息")
    }

    fn redirect_location(&self, url: &Url, deadline: Instant) -> Result<Option<Url>> {
        let response = self.get(
            url.clone(),
            deadline,
            Duration::from_secs(self.config.timeout_seconds),
        )?;
        if response.status().is_redirection() {
            return match response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
            {
                Some(location) => Ok(Some(url.join(location).context("校园网跳转地址无效")?)),
                None => Ok(None),
            };
        }
        let body = Self::body(response)?;
        Ok(extract_redirect(&body).and_then(|location| url.join(&location).ok()))
    }

    pub fn discover_query(&self, deadline: Instant) -> Result<String> {
        let mut candidates = vec![self.root.to_string()];
        candidates.extend(self.config.discovery_urls.iter().cloned());
        candidates.extend(self.config.probe_urls.iter().cloned());
        let mut saw_online = false;
        for raw in candidates {
            let Ok(url) = Url::parse(&raw) else {
                continue;
            };
            let Ok(Some(mut location)) = self.redirect_location(&url, deadline) else {
                continue;
            };
            for _ in 0..3 {
                let path = location.path().to_ascii_lowercase();
                let success_page = path.contains("success.jsp");
                if success_page {
                    saw_online = true;
                }
                if !success_page {
                    if let Some(query) = location.query() {
                        if has_login_parameters(query) {
                            return Ok(query.to_string());
                        }
                    }
                }
                if path.contains("redirectortosuccess.jsp") {
                    let ip = self.local_ip();
                    if !ip.is_empty() {
                        location.query_pairs_mut().append_pair("wlanuserip", &ip);
                    }
                }
                match self.redirect_location(&location, deadline) {
                    Ok(Some(next)) => location = next,
                    _ => break,
                }
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        bail!(if saw_online {
            "网关仅返回现有在线会话，未提供可登录的设备参数；请先检查当前会话。"
        } else {
            "未取得校园网设备参数，请连接校园网络，并检查学校网关、认证发现地址。"
        })
    }

    fn prime_cookies(&self, account: &StoredAccount) {
        let service = if account.service.is_empty() {
            &self.config.service
        } else {
            &account.service
        };
        let group = if account.user_group.is_empty() {
            &self.config.user_group
        } else {
            &account.user_group
        };
        for (name, value) in [
            ("EPORTAL_COOKIE_SAVEPASSWORD", "true".to_owned()),
            ("EPORTAL_COOKIE_NEWV", "true".to_owned()),
            ("EPORTAL_AUTO_LAND", String::new()),
            ("EPORTAL_COOKIE_DOMAIN", "undefined".to_owned()),
            ("EPORTAL_COOKIE_OPERATORPWD", String::new()),
            (
                "EPORTAL_COOKIE_USERNAME",
                quote_component(&account.username),
            ),
            (
                "EPORTAL_COOKIE_PASSWORD",
                quote_component(&account.password),
            ),
            ("EPORTAL_COOKIE_SERVER", quote_component(service)),
            ("EPORTAL_COOKIE_SERVER_NAME", quote_component(service)),
            ("EPORTAL_USER_GROUP", quote_component(group)),
        ] {
            self.jar.add_cookie_str(
                &format!(
                    "{name}={value}; Path=/eportal{}",
                    if self.root.scheme() == "https" {
                        "; Secure"
                    } else {
                        ""
                    }
                ),
                &self.base,
            );
        }
    }

    pub fn login(
        &self,
        account: &StoredAccount,
        query: &str,
        deadline: Instant,
    ) -> Result<(bool, String)> {
        if self.config.password_encrypt {
            bail!("当前 ePortal 网关要求 RSA 加密，此版本暂不支持。")
        }
        let service = if account.service.is_empty() {
            &self.config.service
        } else {
            &account.service
        };
        let body = login_body(&account.username, &account.password, service, query)?;
        self.prime_cookies(account);
        let page = self.base.join(&format!("index.jsp?{query}"))?;
        let _ = self.get(
            page.clone(),
            deadline,
            Duration::from_secs(self.config.timeout_seconds),
        );
        let response = self
            .client
            .post(self.base.join("InterFace.do?method=login")?)
            .timeout(Self::remaining(
                deadline,
                Duration::from_secs(self.config.timeout_seconds),
            )?)
            .header(
                CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=UTF-8",
            )
            .header("Referer", page.as_str())
            .header("Origin", self.root.origin().ascii_serialization())
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .body(body)
            .send()
            .map_err(|error| anyhow!(network_error(&error)))?;
        if !response.status().is_success() {
            bail!("认证网关拒绝了请求，请稍后重试。")
        }
        let value: Value =
            serde_json::from_str(&Self::body(response)?).context("认证网关返回了无效登录响应")?;
        let ok = value.get("result").and_then(Value::as_str) == Some("success");
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(if ok {
                "认证已提交。"
            } else {
                "网关拒绝认证，请检查账号和服务。"
            })
            .to_owned();
        Ok((ok, message))
    }

    pub fn logout(&self, deadline: Instant) -> Result<(bool, String)> {
        let info = self.online_info(deadline, true).ok();
        let index = info
            .as_ref()
            .and_then(|value| value.get("userIndex"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut url = self.base.join("InterFace.do?method=logout")?;
        if !index.is_empty() {
            url.query_pairs_mut().append_pair("userIndex", index);
        }
        let response = self.get(
            url,
            deadline,
            Duration::from_secs(self.config.timeout_seconds),
        )?;
        let value: Value =
            serde_json::from_str(&Self::body(response)?).context("网关未返回有效注销信息")?;
        let ok = value.get("result").and_then(Value::as_str) == Some("success");
        Ok((
            ok,
            value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(if ok {
                    "已注销。"
                } else {
                    "注销未成功。"
                })
                .to_owned(),
        ))
    }

    pub fn services(&self, deadline: Instant) -> Result<Vec<Service>> {
        let response = self.get(
            self.base.join("InterFace.do?method=getServices")?,
            deadline,
            Duration::from_secs(self.config.timeout_seconds),
        )?;
        let body = Self::body(response)?;
        let mut services = Vec::new();
        if let Ok(value) = serde_json::from_str::<Value>(&body) {
            if let Some(values) = value
                .as_array()
                .or_else(|| value.get("services").and_then(Value::as_array))
            {
                for item in values {
                    let value = item
                        .get("serviceName")
                        .or_else(|| item.get("value"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let name = item
                        .get("showName")
                        .or_else(|| item.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or(value);
                    if !value.is_empty() {
                        services.push(Service {
                            value: value.into(),
                            name: name.into(),
                        });
                    }
                }
            }
        }
        for part in body.split("selectService(").skip(1) {
            let mut quotes = part.split('\'');
            quotes.next();
            let value = quotes.next().unwrap_or("");
            quotes.next();
            let name = quotes.next().unwrap_or(value);
            if !value.is_empty() && !services.iter().any(|item| item.value == value) {
                services.push(Service {
                    value: value.into(),
                    name: name.into(),
                });
            }
        }
        Ok(services)
    }

    pub fn local_ip(&self) -> String {
        let Some(host) = self.root.host_str() else {
            return String::new();
        };
        let Ok(ip) = host.parse::<IpAddr>() else {
            return String::new();
        };
        let binding = if ip.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
        let Ok(socket) = UdpSocket::bind(binding) else {
            return String::new();
        };
        if socket
            .connect(SocketAddr::new(
                ip,
                self.root.port_or_known_default().unwrap_or(80),
            ))
            .is_err()
        {
            return String::new();
        }
        socket
            .local_addr()
            .map(|address| address.ip().to_string())
            .unwrap_or_default()
    }
}

fn portal_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("eportal") || lower.contains("wlanuserip") || text.contains("上网认证")
}

fn known_probe_matches(url: &Url, body: &str) -> bool {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    // A generate_204 endpoint returning HTTP 200 is a captive response even
    // when the page does not contain the ePortal product name.
    if path.contains("generate_204") || path.contains("gen_204") {
        return false;
    }
    if host == "www.msftconnecttest.com" && path == "/connecttest.txt" {
        return body.trim() == "Microsoft Connect Test";
    }
    if host == "www.msftncsi.com" && path == "/ncsi.txt" {
        return body.trim() == "Microsoft NCSI";
    }
    if host == "www.baidu.com" || host == "baidu.com" {
        return body.contains("百度一下")
            || body.contains("百度首页")
            || body.contains("www.baidu.com/img/");
    }
    false
}

fn network_error(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "网络请求超时，请检查校园网连接。"
    } else if error.is_connect() {
        "无法连接学校认证网关或联网检测目标。"
    } else {
        "网络请求失败，请检查学校网关和网络设置。"
    }
}

pub fn extract_redirect(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    for marker in [
        "location.href",
        "location.replace",
        "window.location",
        "location=",
    ] {
        if let Some(index) = lower.find(marker) {
            let rest = &body[index + marker.len()..];
            if let Some(quote_at) = rest.find(['\'', '"']) {
                let quote = rest.as_bytes()[quote_at] as char;
                if let Some(end) = rest[quote_at + 1..].find(quote) {
                    return Some(rest[quote_at + 1..quote_at + 1 + end].replace("&amp;", "&"));
                }
            }
        }
    }
    if lower.contains("http-equiv") && lower.contains("refresh") {
        if let Some(index) = lower.find("url=") {
            let rest = body[index + 4..].trim_start_matches(['\'', '"', ' ']);
            let end = rest.find(['\'', '"', '>']).unwrap_or(rest.len());
            return Some(rest[..end].trim().replace("&amp;", "&"));
        }
    }
    for path in ["/eportal/index.jsp?", "/eportal/login.jsp?"] {
        if let Some(index) = lower.find(path) {
            let before = &body[..index];
            let start = before
                .rfind(['\'', '"', ' ', '<', '>'])
                .map(|v| v + 1)
                .unwrap_or(index);
            let rest = &body[start..];
            let end = rest.find(['\'', '"', ' ', '<', '>']).unwrap_or(rest.len());
            return Some(rest[..end].replace("&amp;", "&"));
        }
    }
    None
}

pub(crate) fn status(cfg: &ConnectionConfig) -> Status {
    let mut result = Status {
        online: None,
        network_online: None,
        portal_online: None,
        reason: String::new(),
        message: String::new(),
        username: String::new(),
        service: String::new(),
        local_ip: String::new(),
        checked_at: Local::now().timestamp(),
    };
    let session = match PortalSession::new(cfg) {
        Ok(session) => session,
        Err(error) => {
            result.reason = error.to_string();
            result.message = "请检查学校认证设置。".into();
            return result;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(8);
    let (online, reason) = session.probe_state(deadline, true);
    result.online = online;
    result.network_online = online;
    result.reason = reason;
    if let Ok(info) = session.online_info(deadline, true) {
        result.portal_online = match info
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "success" => Some(true),
            "fail" | "failed" | "failure" | "error" | "offline" => Some(false),
            _ => None,
        };
        result.username = info
            .get("userId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        result.service = info
            .get("service")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
    }
    result.local_ip = session.local_ip();
    result.message = match (result.network_online, result.portal_online) {
        (Some(true), _) => "互联网连接正常。",
        (Some(false), Some(true)) => "校园网会话在线，但互联网检测未通过。",
        (Some(false), Some(false)) => "校园网尚未认证，请连接。",
        _ => "未连接互联网，请确认已接入校园网络。",
    }
    .into();
    result
}
