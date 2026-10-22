use axum::{Router, response::Html, routing::get};

const INDEX_HTML: &str = include_str!("../../dashboard/index.html");
const APP_JS: &str = include_str!("../../dashboard/app.js");
const PIXELS_JS: &str = include_str!("../../dashboard/pixels.js");

pub fn routes<S: Clone + Send + Sync + 'static>(dev: bool) -> Router<S> {
    if dev {
        // Dev mode: serve from disk (edit files, refresh browser, no rebuild).
        Router::new()
            .route("/api/version", get(version))
            .fallback_service(
                tower_http::services::ServeDir::new("dashboard")
                    .fallback(tower_http::services::ServeFile::new("dashboard/index.html")),
            )
    } else {
        // Production: embedded in binary. Explicit script routes for correct
        // content-type; everything else falls back to index.html (SPA routing).
        Router::new()
            .route("/app.js", get(|| js(APP_JS)))
            .route("/pixels.js", get(|| js(PIXELS_JS)))
            .route("/api/version", get(version))
            .fallback(get(index))
    }
}

// Render sets RENDER_GIT_COMMIT on the running service.
async fn version() -> String {
    std::env::var("RENDER_GIT_COMMIT")
        .map(|c| c.chars().take(7).collect())
        .unwrap_or_else(|_| "unknown".to_string())
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn js(body: &'static str) -> ([(&'static str, &'static str); 1], &'static str) {
    ([("content-type", "application/javascript")], body)
}
