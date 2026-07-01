use axum::{
    routing::get,
    Router,
    http::{Method, Request, header},
};
use tower::ServiceExt;
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
    let allow_methods = response.headers().get(header::ACCESS_CONTROL_ALLOW_METHODS).unwrap();
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

    // CORS middleware returns 200 OK for failed preflight but without the CORS headers or with mismatch
    // Actually tower-http CORS returns NO_CONTENT or OK depending on config, but if it doesn't match it doesn't return the allow headers
    assert!(response_post.headers().get(header::ACCESS_CONTROL_ALLOW_METHODS).is_none());
}
