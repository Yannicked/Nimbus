use axum::{
    http::{header, Method, Request},
    routing::get,
    Router,
};
use tower::util::ServiceExt;
use tower_http::cors::CorsLayer;

#[tokio::test]
async fn test_cors_policy() {
    let app = Router::new()
        .route("/api/test", get(|| async { "ok" }))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([Method::GET]),
        );

    // Test Preflight (OPTIONS) request
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/test")
                .header(header::ORIGIN, "https://example.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let allow_methods = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_METHODS)
        .unwrap();
    assert_eq!(allow_methods, "GET");

    // Test that POST is not allowed in preflight
    let response_post = Router::new()
        .route("/api/test", get(|| async { "ok" }))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([Method::GET]),
        )
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/test")
                .header(header::ORIGIN, "https://example.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Test that preflight reflects allowed methods
    let allow_methods_post = response_post
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_METHODS);
    if let Some(methods) = allow_methods_post {
        assert_eq!(methods, "GET");
    }
}
