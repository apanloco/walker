use axum::{Router, response::Html, routing::get};

const INDEX_HTML: &str = include_str!("../../dashboard/index.html");
const APP_JS: &str = include_str!("../../dashboard/app.js");

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(js))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn js() -> ([(&'static str, &'static str); 1], &'static str) {
    ([("content-type", "application/javascript")], APP_JS)
}
