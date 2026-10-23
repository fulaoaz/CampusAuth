//! Headless HTTP management service. Desktop applications do not start a listener.

use crate::{
    core::Core,
    server_auth::{self, Administrator, LoginError},
};
use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, sync::Semaphore, task::spawn_blocking};

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub host: String,
    pub port: u16,
    pub token: Option<String>,
    /// Exact external HTTPS origin when running behind a TLS reverse proxy.
    pub public_url: Option<String>,
    pub allow_insecure_lan: bool,
}
#[derive(Clone)]
struct AppState {
    core: Arc<Core>,
    guard: Arc<RequestGuard>,
    administrator: Arc<Administrator>,
    operations: Arc<Semaphore>,
    password_operations: Arc<Semaphore>,
}
struct RequestGuard {
    authorities: Vec<String>,
    public_origin: Option<String>,
    token: Option<String>,
    wildcard_port: Option<u16>,
    secure_cookie: bool,
    private_peers_only: bool,
}
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"ok": false, "error": self.1}))).into_response()
    }
}
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut different = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        different |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    different == 0
}
fn normalized_authority(authority: &str) -> Option<String> {
    if authority.is_empty()
        || authority
            .chars()
            .any(|ch| ch.is_whitespace() || "/\\@#?".contains(ch))
    {
        return None;
    }
    Some(authority.to_ascii_lowercase())
}
fn origin_authority(origin: &str) -> Option<String> {
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))?;
    if rest.contains(['/', '?', '#']) {
        return None;
    }
    normalized_authority(rest)
}
fn supplied_cookie(headers: &HeaderMap) -> Option<&str> {
    server_auth::cookie_token(
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok()),
    )
}
fn secure_cookie(state: &AppState, headers: &HeaderMap) -> bool {
    state.guard.secure_cookie
        && state.guard.public_origin.as_ref().is_some_and(|origin| {
            let public = origin_authority(origin);
            let host = headers
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .and_then(normalized_authority);
            host == public
                || headers
                    .get(header::ORIGIN)
                    .and_then(|value| value.to_str().ok())
                    == Some(origin.as_str())
        })
}
fn authenticated(state: &AppState, headers: &HeaderMap) -> bool {
    if let Some(expected) = &state.guard.token {
        if let Some(supplied) = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
        {
            if constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
                return true;
            }
        }
    }
    supplied_cookie(headers).is_some_and(|token| state.administrator.authenticated(token))
}
async fn protect(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let denied = |status, text: &str| ApiError(status, text.to_owned()).into_response();
    if state.guard.private_peers_only {
        let private = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|peer| match peer.0.ip() {
                IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
                IpAddr::V6(ip) => {
                    ip.is_loopback()
                        || ip.is_unique_local()
                        || ip.is_unicast_link_local()
                        || ip.to_ipv4_mapped().is_some_and(|v4| {
                            v4.is_loopback() || v4.is_private() || v4.is_link_local()
                        })
                }
            });
        if !private {
            return denied(
                StatusCode::FORBIDDEN,
                "局域网访问模式只允许本机和私有网络客户端",
            );
        }
    }
    let headers = request.headers();
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(normalized_authority)
    else {
        return denied(StatusCode::BAD_REQUEST, "请求缺少有效的 Host");
    };
    let wildcard_ip = state.guard.wildcard_port.is_some_and(|port| {
        host.parse::<axum::http::uri::Authority>()
            .ok()
            .is_some_and(|authority| {
                authority.port_u16() == Some(port)
                    && authority
                        .host()
                        .trim_matches(['[', ']'])
                        .parse::<IpAddr>()
                        .is_ok_and(|ip| !ip.is_unspecified())
            })
    });
    if !wildcard_ip
        && !state
            .guard
            .authorities
            .iter()
            .any(|allowed| allowed == &host)
    {
        return denied(StatusCode::FORBIDDEN, "此 Host 不在允许列表中");
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let Some(origin) = origin.to_str().ok() else {
            return denied(StatusCode::FORBIDDEN, "请求来源无效");
        };
        let authority_matches = origin_authority(origin).as_deref() == Some(host.as_str());
        let public_matches = state.guard.public_origin.as_deref() == Some(origin);
        if !authority_matches && !public_matches {
            return denied(StatusCode::FORBIDDEN, "不允许跨站请求");
        }
        // Trust only the configured HTTPS origin; never X-Forwarded-* supplied by clients.
        if state
            .guard
            .public_origin
            .as_ref()
            .is_some_and(|public| origin_authority(public).as_deref() == Some(host.as_str()))
            && !public_matches
        {
            return denied(StatusCode::FORBIDDEN, "远程访问需要配置的 HTTPS 来源");
        }
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|h| h == "cross-site")
    {
        return denied(StatusCode::FORBIDDEN, "不允许跨站请求");
    }
    let path = request.uri().path();
    if path.starts_with("/api/") {
        if !matches!(path, "/api/login" | "/api/session") && !authenticated(&state, headers) {
            return denied(StatusCode::UNAUTHORIZED, "请先使用管理密码登录");
        }
        if !matches!(*request.method(), Method::GET | Method::HEAD) {
            let json_content = headers
                .get(header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .is_some_and(|h| h.split(';').next().unwrap_or("").trim() == "application/json");
            if !json_content {
                return denied(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "请使用 application/json",
                );
            }
        }
    }
    let request = if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let (parts, body) = request.into_parts();
        let body = match tokio::time::timeout(Duration::from_secs(5), to_bytes(body, 64 * 1024))
            .await
        {
            Ok(Ok(body)) => body,
            Ok(Err(_)) => return denied(StatusCode::PAYLOAD_TOO_LARGE, "请求内容不能超过 64 KiB"),
            Err(_) => return denied(StatusCode::REQUEST_TIMEOUT, "读取请求内容超时"),
        };
        Request::from_parts(parts, Body::from(body))
    } else {
        request
    };
    let mut response = next.run(request).await;
    let output = response.headers_mut();
    output.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    output.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    output.insert("x-frame-options", HeaderValue::from_static("DENY"));
    output.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}
async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let setup_required = match state.administrator.configured() {
        Ok(configured) => !configured,
        Err(_) => {
            return ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "管理员配置不可用，请在本机 TUI 重设管理密码".into(),
            )
            .into_response();
        }
    };
    let authenticated = authenticated(&state, &headers);
    Json(json!({"ok":true,"authenticated":authenticated,"setup_required":setup_required,"username":if authenticated {Some("admin")} else {None}})).into_response()
}
#[derive(Deserialize)]
struct LoginInput {
    password: String,
}
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<LoginInput>,
) -> Response {
    let Ok(permit) = state.password_operations.clone().try_acquire_owned() else {
        return ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "正在验证管理密码，请稍后重试".into(),
        )
        .into_response();
    };
    let administrator = state.administrator.clone();
    let result = spawn_blocking(move || {
        let _permit = permit;
        administrator.login(&input.password)
    })
    .await;
    let token = match result {
        Ok(Ok(token)) => token,
        Ok(Err(LoginError::NotConfigured)) => {
            return ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "尚未设置管理密码。请在服务器本机运行 campus-auth password set 或进入 TUI 设置"
                    .into(),
            )
            .into_response();
        }
        Ok(Err(LoginError::Invalid)) => {
            return ApiError(StatusCode::UNAUTHORIZED, "管理密码错误".into()).into_response();
        }
        Ok(Err(LoginError::RateLimited)) => {
            return ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "密码尝试过于频繁，请在 30 秒后重试".into(),
            )
            .into_response();
        }
        _ => {
            return ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "管理认证暂时不可用，请在本机检查配置".into(),
            )
            .into_response();
        }
    };
    let secure = if secure_cookie(&state, &headers) {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{secure}",
        server_auth::SESSION_COOKIE,
        server_auth::SESSION_SECONDS
    );
    let mut response =
        Json(json!({"ok":true,"authenticated":true,"setup_required":false,"username":"admin"}))
            .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("safe random session"),
    );
    response
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = supplied_cookie(&headers) {
        state.administrator.logout(token);
    }
    let secure = if secure_cookie(&state, &headers) {
        "; Secure"
    } else {
        ""
    };
    let mut response = Json(json!({"ok":true,"authenticated":false})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{secure}",
            server_auth::SESSION_COOKIE
        ))
        .expect("safe cookie"),
    );
    response
}
async fn api(
    State(state): State<AppState>,
    Path(method): Path<String>,
    request: Request,
) -> Response {
    let http_method = request.method().clone();
    let params = if matches!(http_method, Method::GET | Method::HEAD) {
        None
    } else {
        let bytes = match to_bytes(request.into_body(), 64 * 1024).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return ApiError(StatusCode::BAD_REQUEST, "无法读取请求内容".into())
                    .into_response();
            }
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => Some(value),
            Err(_) => {
                return ApiError(StatusCode::BAD_REQUEST, "请求 JSON 格式无效".into())
                    .into_response();
            }
        }
    };
    if !matches!(http_method, Method::GET | Method::HEAD | Method::POST) {
        return ApiError(
            StatusCode::METHOD_NOT_ALLOWED,
            "此接口仅支持 GET 或 POST".into(),
        )
        .into_response();
    }
    let readonly = [
        "health",
        "status",
        "snapshot",
        "accounts",
        "services",
        "logs",
        "profiles",
        "power",
        "app/info",
        "diagnostics",
        "updates/check",
    ];
    let mutation = method.ends_with("/save")
        || method.ends_with("/delete")
        || method.ends_with("/reorder")
        || method.ends_with("/set")
        || matches!(
            method.as_str(),
            "connect" | "disconnect" | "diagnostics/export" | "power/cancel" | "reconnect/tick"
        );
    if (http_method == Method::GET || http_method == Method::HEAD) && mutation {
        return ApiError(
            StatusCode::METHOD_NOT_ALLOWED,
            "修改配置需要 POST 请求".into(),
        )
        .into_response();
    }
    if http_method == Method::POST && readonly.contains(&method.as_str()) {
        return ApiError(StatusCode::METHOD_NOT_ALLOWED, "此接口仅支持读取".into()).into_response();
    }
    if matches!(method.as_str(), "power/install" | "power/remove") {
        return Json(json!({"ok":false,"permission_error":true,"message":"服务器 WebUI 不执行系统电源操作。请在服务器本机进入 TUI，先预览计划，再明确确认应用或删除。","admin_command":format!("sudo campus-auth --data-dir '{}' tui",state.core.data_dir().display())})).into_response();
    }
    if method.starts_with("power/")
        && !matches!(
            method.as_str(),
            "power/preview" | "power/config" | "power/countdown" | "power/cancel" | "power/status"
        )
    {
        return ApiError(
            StatusCode::FORBIDDEN,
            "电源计划的系统安装和删除请在本机 TUI 预览并确认执行".into(),
        )
        .into_response();
    }
    if method == "power/config" && http_method == Method::POST {
        return ApiError(
            StatusCode::FORBIDDEN,
            "服务器 WebUI 仅查看电源计划；修改请在本机 TUI 操作".into(),
        )
        .into_response();
    }
    let Ok(permit) = state.operations.clone().try_acquire_owned() else {
        return ApiError(StatusCode::TOO_MANY_REQUESTS, "服务繁忙，请稍后重试".into())
            .into_response();
    };
    let core = state.core;
    if method == "health" {
        return Json(json!({"ok":true,"version":env!("CARGO_PKG_VERSION")})).into_response();
    }
    let result = spawn_blocking(move || {
        let _permit = permit;
        crate::api::dispatch(&core, &method, params)
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => ApiError(StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        Err(_) => {
            ApiError(StatusCode::INTERNAL_SERVER_ERROR, "操作未能完成".into()).into_response()
        }
    }
}
async fn asset(request: Request) -> Response {
    let path = request.uri().path().trim_start_matches('/');
    if path.split('/').any(|part| part == "..") || path.contains('\\') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some((bytes, mime)) = crate::web_assets::lookup(path) {
        return ([(header::CONTENT_TYPE, mime)], bytes).into_response();
    }
    if !path.contains('.') && !path.starts_with("api/") {
        if let Some((bytes, mime)) = crate::web_assets::lookup("index.html") {
            return ([(header::CONTENT_TYPE, mime)], bytes).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}
async fn reconnect_loop(core: Arc<Core>) {
    let mut last = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let read_core = core.clone();
        let Ok(Ok(settings)) = spawn_blocking(move || read_core.reconnect_settings()).await else {
            continue;
        };
        if !settings.enabled
            || last.elapsed() < Duration::from_secs(settings.interval_seconds.max(10))
        {
            continue;
        }
        last = Instant::now();
        let reconnect_core = core.clone();
        let _ = spawn_blocking(move || reconnect_core.reconnect_tick()).await;
    }
}
pub async fn run_background(core: Arc<Core>) -> Result<()> {
    println!(
        "CampusAuth 后台认证已启动；当前模式不监听 HTTP。使用同一数据目录进入 TUI 即可管理配置。"
    );
    tokio::select! { _ = reconnect_loop(core) => {}, _ = shutdown() => {} }
    Ok(())
}
pub async fn run(core: Arc<Core>, options: ServerOptions) -> Result<()> {
    let ip: IpAddr = options
        .host
        .parse()
        .context("监听地址必须是 IPv4 或 IPv6 地址")?;
    let token = options.token.filter(|token| !token.trim().is_empty());
    if token.as_ref().is_some_and(|value| value.len() < 24) {
        bail!("兼容 API 令牌需要至少 24 个字符");
    }
    let public_origin = options
        .public_url
        .map(|text| -> Result<String> {
            let url = url::Url::parse(&text).context("远程访问 URL 无效")?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                bail!("远程访问地址必须是完整 HTTPS 来源，例如 https://auth.example.com");
            }
            Ok(url.origin().ascii_serialization())
        })
        .transpose()?;
    if !ip.is_loopback() && !options.allow_insecure_lan {
        bail!(
            "局域网监听需要明确使用 --allow-insecure-lan；默认建议保留 127.0.0.1 并使用 HTTPS 代理或 SSH 隧道"
        );
    }
    if !ip.is_loopback() {
        eprintln!(
            "已明确开启局域网 HTTP 访问。管理密码和操作将在局域网内明文传输，请仅在可信网络使用；互联网访问请使用 HTTPS 代理。"
        );
    }
    let listener = TcpListener::bind(SocketAddr::new(ip, options.port))
        .await
        .context("无法监听 WebUI 端口")?;
    let port = listener.local_addr()?.port();
    let authority = if ip.is_ipv6() {
        format!("[{ip}]:{port}")
    } else {
        format!("{ip}:{port}")
    };
    let mut authorities = vec![authority.clone()];
    if ip.is_loopback() || ip.is_unspecified() {
        authorities.extend([
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ]);
    }
    if let Some(public) = &public_origin {
        authorities.push(origin_authority(public).context("远程来源无效")?);
    }
    let state = AppState {
        administrator: Arc::new(Administrator::new(core.data_dir())),
        core,
        guard: Arc::new(RequestGuard {
            authorities,
            secure_cookie: public_origin.is_some(),
            public_origin,
            token,
            wildcard_port: ip.is_unspecified().then_some(port),
            private_peers_only: !ip.is_loopback(),
        }),
        operations: Arc::new(Semaphore::new(8)),
        password_operations: Arc::new(Semaphore::new(2)),
    };
    let reconnect_task = tokio::spawn(reconnect_loop(state.core.clone()));
    let router = Router::new()
        .route("/api/login", post(login))
        .route("/api/session", get(session))
        .route("/api/logout", post(logout))
        .route("/api/{*method}", any(api))
        .fallback(asset)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), protect))
        .with_state(state);
    println!("CampusAuth WebUI 已监听 http://{authority}；管理员首次密码请在本机 TUI 设置。");
    let result = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await;
    reconnect_task.abort();
    result.context("WebUI 服务异常结束")
}
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install signal handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
