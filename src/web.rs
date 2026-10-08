use crate::{
    auth::{Auth, COOKIE, cookie_token},
    config::Config,
    jobs::Queue,
    model::{Chat, ChatSummary, Job, Media},
    store::{MAX_IMAGE_BYTES, Store, media_record},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Response, Sse,
        sse::{Event as SseEvent, KeepAlive},
    },
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio_util::io::ReaderStream;

#[derive(Clone)]
pub struct App {
    pub store: Arc<Store>,
    pub auth: Arc<Auth>,
    pub queue: Arc<Queue>,
    pub config: Config,
}

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);
impl ApiError {
    fn bad(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "Project or image not found.".into())
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        tracing::warn!(%error, "request failed");
        Self(StatusCode::BAD_REQUEST, error.to_string())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}

pub fn router(app: App) -> Router {
    let private = Router::new()
        .route("/api/status", get(status))
        .route("/api/logout", post(logout))
        .route("/api/chats", get(list_chats).post(create_chat))
        .route("/api/chats/{id}", get(read_chat).patch(rename_chat))
        .route("/api/chats/{id}/export", get(export_chat))
        .route("/api/chats/{id}/messages", post(send_message))
        .route("/api/chats/{id}/uploads", post(upload))
        .route("/api/chats/{id}/jobs/{job_id}/cancel", post(cancel))
        .route("/api/chats/{id}/media/{media_id}", get(image))
        .route("/api/events", get(events))
        .layer(middleware::from_fn_with_state(app.clone(), authenticate));
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../public/index.html")) }),
        )
        .route(
            "/app.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../public/app.css"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../public/app.js"),
                )
            }),
        )
        .route("/api/login", post(login).layer(DefaultBodyLimit::max(4096)))
        .route(
            "/logo.svg",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/svg+xml")],
                    include_str!("../public/logo.svg"),
                )
            }),
        )
        .route(
            "/instrument-sans.ttf",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "font/ttf")],
                    include_bytes!("../public/instrument-sans.ttf").as_slice(),
                )
            }),
        )
        .merge(private)
        .fallback(|| async { ApiError::missing() })
        .layer(DefaultBodyLimit::max(MAX_IMAGE_BYTES + 64 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), request_policy))
        .layer(middleware::from_fn(response_policy))
        .with_state(app)
}

async fn response_policy(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

fn allowed_origin(config: &Config, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    if let Some(expected) = &config.public_origin {
        return origin == expected;
    }
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    // Without a configured public origin only the local UI is accepted.
    let local = host.starts_with("localhost:")
        || host.starts_with("127.0.0.1:")
        || host.starts_with("[::1]:");
    local && (origin == format!("http://{host}") || origin == format!("https://{host}"))
}

async fn request_policy(State(app): State<App>, request: Request, next: Next) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD)
        && (request
            .headers()
            .get("x-pocket-request")
            .and_then(|h| h.to_str().ok())
            != Some("1")
            || !allowed_origin(&app.config, request.headers()))
    {
        return ApiError(
            StatusCode::FORBIDDEN,
            "Request origin rejected. Set --public-origin to your ngrok HTTPS origin.".into(),
        )
        .into_response();
    }
    next.run(request).await
}

fn request_token(headers: &HeaderMap) -> Option<&str> {
    cookie_token(headers.get(header::COOKIE)?.to_str().ok()?)
}

async fn authenticate(State(app): State<App>, request: Request, next: Next) -> Response {
    match request_token(request.headers()) {
        Some(token) if app.auth.valid(token).await => next.run(request).await,
        _ => ApiError(
            StatusCode::UNAUTHORIZED,
            "Sign in to your creative desk.".into(),
        )
        .into_response(),
    }
}

#[derive(Deserialize)]
struct Login {
    key: String,
}
async fn login(State(app): State<App>, Json(body): Json<Login>) -> Result<Response, ApiError> {
    let token = app.auth.login(body.key.trim()).await.map_err(|error| {
        ApiError(
            if error.starts_with("too many") {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::UNAUTHORIZED
            },
            error.into(),
        )
    })?;
    let secure = app
        .config
        .public_origin
        .as_ref()
        .is_some_and(|s| s.starts_with("https://"));
    let cookie = format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=604800{}",
        if secure { "; Secure" } else { "" }
    );
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({"ok":true}))).into_response())
}

async fn logout(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Some(token) = request_token(&headers) {
        app.auth.logout(token).await;
    }
    (
        [(
            header::SET_COOKIE,
            format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
        )],
        Json(json!({"ok":true})),
    )
        .into_response()
}

async fn status(State(app): State<App>) -> Json<Value> {
    Json(json!({"version":env!("CARGO_PKG_VERSION"), "runtime":*app.queue.status.read().await}))
}

async fn list_chats(State(app): State<App>) -> Result<Json<Vec<ChatSummary>>, ApiError> {
    Ok(Json(app.store.list().await?))
}

#[derive(Deserialize)]
struct Title {
    title: String,
}
fn clean_title(title: String) -> Result<String, ApiError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 100 || title.chars().any(char::is_control) {
        return Err(ApiError::bad(
            "Use a project title between 1 and 100 characters.",
        ));
    }
    Ok(title.into())
}
async fn create_chat(
    State(app): State<App>,
    Json(body): Json<Title>,
) -> Result<(StatusCode, Json<Chat>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(app.store.create(clean_title(body.title)?).await?),
    ))
}
async fn read_chat(State(app): State<App>, Path(id): Path<String>) -> Result<Json<Chat>, ApiError> {
    Ok(Json(
        app.store.read(&id).await.map_err(|_| ApiError::missing())?,
    ))
}
async fn rename_chat(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<Title>,
) -> Result<Json<Value>, ApiError> {
    let title = clean_title(body.title)?;
    app.store
        .update(&id, |chat| {
            chat.title = title;
            Ok(())
        })
        .await?;
    app.queue.emit(&id, json!({"type":"refresh"}));
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct Prompt {
    text: String,
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default)]
    media_ids: Vec<String>,
}
fn default_mode() -> String {
    "chat".into()
}
async fn send_message(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<Prompt>,
) -> Result<(StatusCode, Json<Job>), ApiError> {
    let text = body.text.trim().to_owned();
    if text.is_empty() || text.len() > 16_000 {
        return Err(ApiError::bad("Write a message between 1 and 16,000 bytes."));
    }
    if !["chat", "generate"].contains(&body.mode.as_str()) {
        return Err(ApiError::bad("Unknown message mode."));
    }
    if body.media_ids.len() > 5 {
        return Err(ApiError::bad("Attach at most five reference images."));
    }
    app.store.read(&id).await.map_err(|_| ApiError::missing())?;
    let job = app
        .queue
        .submit(&app.store, &id, text, body.mode, body.media_ids)
        .await
        .map_err(|error| ApiError(StatusCode::CONFLICT, error.to_string()))?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn upload(
    State(app): State<App>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Media>), ApiError> {
    app.store.read(&id).await.map_err(|_| ApiError::missing())?;
    let field = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::bad("Invalid image upload."))?
        .ok_or_else(|| ApiError::bad("Choose an image to upload."))?;
    if field.name() != Some("file") {
        return Err(ApiError::bad("Expected one file field."));
    }
    let name = field
        .file_name()
        .unwrap_or("Reference image")
        .chars()
        .filter(|c| !c.is_control() && !"/\\\"".contains(*c))
        .take(100)
        .collect::<String>();
    let bytes = field
        .bytes()
        .await
        .map_err(|_| ApiError::bad("Image upload failed or exceeded 12 MB."))?;
    if multipart
        .next_field()
        .await
        .map_err(|_| ApiError::bad("Invalid image upload."))?
        .is_some()
    {
        return Err(ApiError::bad("Upload one image at a time."));
    }
    let media = app
        .store
        .add_media(&id, &bytes, media_record(name, "reference", None, None))
        .await?;
    app.queue.emit(&id, json!({"type":"refresh"}));
    Ok((StatusCode::CREATED, Json(media)))
}

async fn cancel(
    State(app): State<App>,
    Path((id, job_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    app.queue.cancel(&app.store, &id, &job_id).await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct Download {
    download: Option<u8>,
}
async fn image(
    State(app): State<App>,
    Path((id, media_id)): Path<(String, String)>,
    Query(query): Query<Download>,
) -> Result<Response, ApiError> {
    let (media, path) = app
        .store
        .media_path(&id, &media_id)
        .await
        .map_err(|_| ApiError::missing())?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| ApiError::missing())?;
    let disposition = format!(
        "{}; filename=\"{}\"",
        if query.download == Some(1) {
            "attachment"
        } else {
            "inline"
        },
        media.filename
    );
    Ok((
        [
            (header::CONTENT_TYPE, media.mime),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

async fn export_chat(State(app): State<App>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let chat = app.store.read(&id).await.map_err(|_| ApiError::missing())?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"pocket-{id}.json\""),
            ),
        ],
        serde_json::to_vec_pretty(&chat).map_err(anyhow::Error::from)?,
    )
        .into_response())
}

async fn events(State(app): State<App>, headers: HeaderMap) -> Response {
    let token = request_token(&headers).unwrap_or("").to_owned();
    let mut receiver = app.queue.events.subscribe();
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(SseEvent::default().event("update").data("{\"type\":\"resync\"}"));
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                _ = app.queue.shutdown.cancelled() => break,
                _ = interval.tick() => {
                    if !app.auth.valid(&token).await { break; }
                }
                event = receiver.recv() => match event {
                    Ok(event) => {
                        let mut data = event.data;
                        data["chat_id"] = json!(event.chat_id);
                        yield Ok(SseEvent::default().event("update").data(data.to_string()));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        yield Ok(SseEvent::default().event("update").data("{\"type\":\"resync\"}"));
                    }
                    Err(_) => break,
                }
            }
        }
    };
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;
    use clap::Parser;
    use tower::ServiceExt;

    #[tokio::test]
    async fn login_csrf_and_private_images_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        let config =
            Config::parse_from(["test", "--mock", "--public-origin", "https://desk.example"]);
        let (queue, worker) = Queue::start(store.clone(), config.clone());
        let auth = Arc::new(Auth::new("correct-test-key-that-is-at-least-32-chars"));
        let chat = store.create("Private".into()).await.unwrap();
        let media = store
            .add_media(
                &chat.id,
                include_bytes!("../public/mock.png"),
                media_record("Test".into(), "reference", None, None),
            )
            .await
            .unwrap();
        let app = router(App {
            store,
            auth,
            queue: queue.clone(),
            config,
        });
        let private_url = format!("/api/chats/{}/media/{}", chat.id, media.id);
        assert_eq!(
            app.clone()
                .oneshot(
                    HttpRequest::builder()
                        .uri(&private_url)
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let bad = HttpRequest::builder()
            .method("POST")
            .uri("/api/login")
            .header("content-type", "application/json")
            .header("origin", "https://attacker.example")
            .header("x-pocket-request", "1")
            .body(Body::from(
                r#"{"key":"correct-test-key-that-is-at-least-32-chars"}"#,
            ))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(bad).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let login = HttpRequest::builder()
            .method("POST")
            .uri("/api/login")
            .header("content-type", "application/json")
            .header("origin", "https://desk.example")
            .header("x-pocket-request", "1")
            .body(Body::from(
                r#"{"key":"correct-test-key-that-is-at-least-32-chars"}"#,
            ))
            .unwrap();
        let response = app.clone().oneshot(login).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("Secure")
                && cookie.contains("SameSite=Strict")
        );
        let cookie = cookie.split(';').next().unwrap();
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(&private_url)
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        let mutation = HttpRequest::builder()
            .method("POST")
            .uri("/api/chats")
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title":"Blocked"}"#))
            .unwrap();
        assert_eq!(
            app.oneshot(mutation).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        queue.shutdown.cancel();
        worker.await.unwrap();
    }
}
