//! The TCP server's limits: a real listener, since the limits live in the connection setup.
#![allow(clippy::unwrap_used, reason = "tests")]

use axum::Router;
use axum::routing::post;
use tokio::net::TcpListener;

async fn start() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route(
        "/echo",
        post(|body: bytes::Bytes| async move { body.len().to_string() }),
    );
    tokio::spawn(lfm_mirror::serve::serve(
        listener,
        router,
        std::future::pending(),
    ));
    format!("http://{address}")
}

#[tokio::test]
async fn small_bodies_pass_and_large_ones_are_refused() {
    let base = start().await;
    let client = reqwest::Client::new();
    let small = client
        .post(format!("{base}/echo"))
        .body("a=1")
        .send()
        .await
        .unwrap();
    assert_eq!(small.text().await.unwrap(), "3");
    let large = client
        .post(format!("{base}/echo"))
        .body(vec![b'a'; 2 << 20])
        .send()
        .await
        .unwrap();
    assert_eq!(large.status(), 413);
}
