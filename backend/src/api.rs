//! HTTP API, WebSocket live feed, admin endpoints and static frontend hosting.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::header::{
    AUTHORIZATION, CONTENT_TYPE, COOKIE, REFERRER_POLICY, SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::admin::{AdminStore, COOKIE_NAME, InstanceInput, LoginError};
use crate::assets;
use crate::demo;
use crate::model::{MessageView, PeerView};
use crate::runtime_config::RuntimeConfig;
use crate::state::{AppState, ConnectionsResponse, RoutesResponse, ServerEvent};

pub async fn serve(listener: TcpListener, state: Arc<AppState>) -> anyhow::Result<()> {
    let mut shutdown = state.subscribe_shutdown();
    let tick_state = state.clone();
    let mut tick_shutdown = state.subscribe_shutdown();
    let ticker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick_shutdown.changed() => break,
                _ = interval.tick() => tick_state.broadcast_overview(),
            }
        }
    });

    let app = router(state);
    info!("HTTP dashboard/API ready");
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown.changed().await;
    })
    .await;
    ticker.abort();
    result?;
    info!("HTTP server stopped");
    Ok(())
}

fn router(state: Arc<AppState>) -> Router {
    let cfg = state.config.clone();
    let dev_cors = cfg.frontend_dir.is_some();

    // Authentication/session endpoints and the liveness probe stay public.
    // Every business API (dashboard data, admin mutations, WebSocket) is
    // protected by the same cookie session below.
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login", post(admin_login))
        .route("/api/auth/logout", post(admin_logout))
        .route("/api/auth/session", get(admin_session))
        // Compatibility aliases for older clients.
        .route("/api/admin/login", post(admin_login))
        .route("/api/admin/logout", post(admin_logout))
        .route("/api/admin/session", get(admin_session));

    let protected = Router::new()
        .route("/api/overview", get(overview))
        .route("/api/messages", get(messages))
        .route("/api/routes", get(routes))
        .route("/api/peers", get(peers))
        .route("/api/admin/password", post(admin_change_password))
        .route(
            "/api/admin/instances",
            get(admin_instances).post(admin_create_instance),
        )
        .route(
            "/api/admin/instances/{id}",
            put(admin_update_instance).delete(admin_delete_instance),
        )
        .route("/api/admin/settings", put(admin_update_settings))
        .route(
            "/api/admin/runtime",
            get(admin_runtime).put(admin_update_runtime),
        )
        .route("/api/admin/connections", get(admin_connections))
        .route("/api/admin/demo/start", post(admin_demo_start))
        .route("/api/admin/demo/stop", post(admin_demo_stop))
        .route("/ws", get(ws_handler))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let mut app = public.merge(protected);

    // `--frontend-dir` is an explicitly opt-in development override. By default
    // the frontend built by build.rs is embedded in the binary. The fallback is
    // installed before the middleware layers so security headers also apply to
    // static assets.
    match cfg
        .frontend_dir
        .as_ref()
        .filter(|dir| dir.join("index.html").exists())
    {
        Some(dir) => {
            let index = dir.join("index.html");
            let service = ServeDir::new(dir).not_found_service(ServeFile::new(index));
            app = app.fallback_service(service);
        }
        None => {
            app = app.fallback(assets::serve);
        }
    }

    app = app
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(TraceLayer::new_for_http());

    // The Next.js dev server runs on a different port; `--frontend-dir` marks
    // development mode and enables credentialed CORS. Production serves the
    // embedded frontend from the same origin and needs no CORS at all.
    if dev_cors {
        app = app.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::mirror_request())
                .allow_credentials(true)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers([CONTENT_TYPE, AUTHORIZATION, axum::http::header::ACCEPT]),
        );
    }

    app.with_state(state)
}

// ----------------------------------------------------------------------
// Public API
// ----------------------------------------------------------------------

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let overview = state.overview();
    Json(json!({
        "status": "ok",
        "bmpListening": overview.bmp_listening,
        "bmpBind": overview.bmp_bind,
        "httpBind": overview.http_bind,
        "uptimeSeconds": overview.uptime_seconds,
        "activeConnections": overview.active_connections,
        "peersTotal": overview.peers_total,
        "messages": overview.totals.messages,
        "demoRunning": overview.demo.running,
    }))
}

async fn overview(State(state): State<Arc<AppState>>) -> Json<crate::state::Overview> {
    Json(state.overview())
}

#[derive(Debug, Deserialize)]
struct MessagesQuery {
    limit: Option<usize>,
    #[serde(rename = "type")]
    msg_type: Option<String>,
    q: Option<String>,
}

async fn messages(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MessagesQuery>,
) -> Json<Vec<Arc<MessageView>>> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    Json(state.query_messages(limit, query.msg_type.as_deref(), query.q.as_deref()))
}

#[derive(Debug, Deserialize)]
struct RoutesQuery {
    page: Option<usize>,
    limit: Option<usize>,
    q: Option<String>,
    peer: Option<String>,
    router: Option<String>,
}

async fn routes(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RoutesQuery>,
) -> Json<RoutesResponse> {
    // Storage is unbounded; one API response is still deliberately small.
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    Json(state.query_routes(
        page,
        limit,
        query.q.as_deref(),
        query.peer.as_deref(),
        query.router.as_deref(),
    ))
}

async fn peers(State(state): State<Arc<AppState>>) -> Json<Vec<PeerView>> {
    Json(state.peers_list())
}

// ----------------------------------------------------------------------
// Admin auth helpers
// ----------------------------------------------------------------------

type ApiError = (StatusCode, Json<Value>);

fn api_error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": message.into() })))
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == COOKIE_NAME).then(|| value.to_string())
        })
}

async fn require_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(token) = session_token(request.headers()) else {
        return api_error(StatusCode::UNAUTHORIZED, "需要登录").into_response();
    };
    if state.admin.session_valid(&token).is_none() {
        return api_error(StatusCode::UNAUTHORIZED, "会话已过期，请重新登录").into_response();
    }
    next.run(request).await
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let token =
        session_token(headers).ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "需要登录"))?;
    state
        .admin
        .session_valid(&token)
        .map(|_| ())
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "会话已过期，请重新登录"))
}

fn with_cookie(mut response: Response, cookie: String) -> Response {
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    response
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    password: String,
}

async fn admin_login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let store: Arc<AdminStore> = state.admin.clone();
    let password = request.password;
    let ip = addr.ip();
    let result = tokio::task::spawn_blocking(move || store.login(ip, &password))
        .await
        .map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("登录任务失败: {e}"),
            )
        })?;

    match result {
        Ok((token, expires_at)) => {
            let cookie = state.admin.cookie_header(&token, expires_at);
            let response = Json(json!({ "ok": true, "expiresAt": expires_at })).into_response();
            Ok(with_cookie(response, cookie))
        }
        Err(LoginError::Invalid) => Err(api_error(StatusCode::UNAUTHORIZED, "密码不正确")),
        Err(LoginError::Locked(seconds)) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": format!("尝试次数过多，请 {seconds} 秒后重试") })),
        )),
    }
}

async fn admin_logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token(&headers) {
        state.admin.logout(&token);
    }
    let cookie = state.admin.clear_cookie_header();
    with_cookie(Json(json!({ "ok": true })).into_response(), cookie)
}

async fn admin_session(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Json<Value> {
    let expires_at = session_token(&headers).and_then(|token| state.admin.session_valid(&token));
    Json(json!({
        "authenticated": expires_at.is_some(),
        "expiresAt": expires_at,
        "passwordConfigured": state.admin.password_configured(),
    }))
}

// ----------------------------------------------------------------------
// Admin instances & settings
// ----------------------------------------------------------------------

async fn admin_instances(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(json!({
        "instances": state.instance_statuses(),
        "enforceAllowlist": state.admin.enforce_allowlist(),
        "runtime": state.runtime_config(),
    })))
}

async fn admin_create_instance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<InstanceInput>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let store = state.admin.clone();
    let record = tokio::task::spawn_blocking(move || store.create_instance(&input))
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("任务失败: {e}")))?
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    state.broadcast_overview();
    Ok(Json(json!({ "ok": true, "instance": record })))
}

async fn admin_update_instance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<InstanceInput>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let store = state.admin.clone();
    let record = tokio::task::spawn_blocking(move || store.update_instance(&id, &input))
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("任务失败: {e}")))?
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    state.broadcast_overview();
    Ok(Json(json!({ "ok": true, "instance": record })))
}

async fn admin_delete_instance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let store = state.admin.clone();
    tokio::task::spawn_blocking(move || store.delete_instance(&id))
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("任务失败: {e}")))?
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    state.broadcast_overview();
    Ok(Json(json!({ "ok": true })))
}

async fn admin_runtime(State(state): State<Arc<AppState>>) -> Json<RuntimeConfig> {
    Json(state.runtime_config())
}

async fn admin_update_runtime(
    State(state): State<Arc<AppState>>,
    Json(input): Json<RuntimeConfig>,
) -> Result<Json<Value>, ApiError> {
    let config = state
        .update_runtime_config(input)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "ok": true, "runtime": config })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsInput {
    enforce_allowlist: bool,
}

async fn admin_update_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<SettingsInput>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let store = state.admin.clone();
    let enabled = input.enforce_allowlist;
    tokio::task::spawn_blocking(move || store.set_enforce_allowlist(enabled))
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("任务失败: {e}")))?
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    state.broadcast_overview();
    Ok(Json(json!({ "ok": true, "enforceAllowlist": enabled })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PasswordInput {
    current_password: String,
    new_password: String,
}

async fn admin_change_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<PasswordInput>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let store = state.admin.clone();
    let result = tokio::task::spawn_blocking(move || {
        store.change_password(&input.current_password, &input.new_password)
    })
    .await
    .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("任务失败: {e}")))?;

    match result {
        Ok((token, expires_at)) => {
            let cookie = state.admin.cookie_header(&token, expires_at);
            let response = Json(json!({ "ok": true, "expiresAt": expires_at })).into_response();
            Ok(with_cookie(response, cookie))
        }
        Err(message) => Err(api_error(StatusCode::BAD_REQUEST, message)),
    }
}

async fn admin_connections(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ConnectionsResponse>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(state.connections()))
}

async fn admin_demo_start(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    demo::start(&state).map_err(|e| api_error(StatusCode::CONFLICT, e))?;
    state.broadcast_overview();
    Ok(Json(json!({ "ok": true, "running": true })))
}

async fn admin_demo_stop(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let stopped = demo::stop(&state);
    state.broadcast_overview();
    Ok(Json(json!({ "ok": true, "stopped": stopped })))
}

// ----------------------------------------------------------------------
// WebSocket
// ----------------------------------------------------------------------

async fn ws_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let token = session_token(&headers).unwrap_or_default();
    ws.on_upgrade(move |socket| ws_session(socket, state, token))
}

async fn ws_session(socket: WebSocket, state: Arc<AppState>, token: String) {
    let (mut sender, mut receiver) = socket.split();
    let mut shutdown = state.subscribe_shutdown();
    let mut events = state.subscribe();
    let mut auth_check = tokio::time::interval(Duration::from_secs(30));
    auth_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // The receiver is subscribed before the snapshot, so no event can slip between.
    if !send_event(
        &mut sender,
        &ServerEvent::Overview(Arc::new(state.overview())),
    )
    .await
    {
        return;
    }
    for message in state.recent_messages(60) {
        if !send_event(&mut sender, &ServerEvent::Message(message)).await {
            return;
        }
    }

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = auth_check.tick() => {
                if state.admin.session_valid(&token).is_none() {
                    break;
                }
            }
            event = events.recv() => match event {
                Ok(event) => {
                    if !send_event(&mut sender, &event).await {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    if !send_event(&mut sender, &ServerEvent::Lagged { skipped }).await {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = receiver.next() => match incoming {
                Some(Ok(Message::Ping(payload))) => {
                    if tokio::time::timeout(
                        Duration::from_secs(5),
                        sender.send(Message::Pong(payload)),
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
    }
    let _ = sender.close().await;
}

async fn send_event(sender: &mut SplitSink<WebSocket, Message>, event: &ServerEvent) -> bool {
    let Ok(text) = serde_json::to_string(event) else {
        return false;
    };
    tokio::time::timeout(
        Duration::from_secs(5),
        sender.send(Message::Text(text.into())),
    )
    .await
    .map(|result| result.is_ok())
    .unwrap_or(false)
}
