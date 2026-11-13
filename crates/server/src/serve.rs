//! Serving the router over TCP with limits that `axum::serve` does not set: a client must send
//! its request head within [`HEADER_TIMEOUT`], a handler must answer within
//! [`REQUEST_TIMEOUT`], and a request body may hold at most [`BODY_LIMIT`].

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::extract::connect_info::ConnectInfo;
use axum::http::StatusCode;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tower_http::timeout::TimeoutLayer;

/// Time to receive a request's head; a slow client cannot hold a connection open for free.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);

/// Time for a handler to produce its response head. Pass-through misses wait on Last.fm's
/// retries, so it is generous; an export's body streams after it and is not limited.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// The largest request body: a batch of 50 scrobbles is a few kilobytes.
const BODY_LIMIT: usize = 1 << 20;

/// A pause after a failed accept, such as running out of file descriptors, so the loop does
/// not spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Serves `router` on `listener` until `shutdown` completes, then waits for open requests.
pub async fn serve(listener: TcpListener, router: Router, shutdown: impl Future<Output = ()>) {
    let router =
        router
            .layer(DefaultBodyLimit::max(BODY_LIMIT))
            .layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                REQUEST_TIMEOUT,
            ));
    let mut builder = Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT);
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!(%error, "accept failed");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let service = router
            .clone()
            .layer(axum::Extension(ConnectInfo::<SocketAddr>(peer)));
        let service = TowerToHyperService::new(service);
        let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
        let connection = graceful.watch(connection.into_owned());
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "connection ended with an error");
            }
        });
    }
    graceful.shutdown().await;
}
