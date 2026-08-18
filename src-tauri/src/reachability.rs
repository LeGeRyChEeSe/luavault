//! Steam reachability probe used by the explicit startup and retry checks.
//!
//! This module deliberately owns no timer or background task: callers invoke
//! [`probe_steam`] once, then publish its result.  A failed game-detail request
//! remains a local failure of that request; it does not alter this signal.

use reqwest::Client;
use serde::Serialize;
use std::time::{Duration, Instant};

/// Steam Store's stable root endpoint.  It gives the application one common
/// point of truth without making automated tests depend on the public network.
pub const STEAM_REACHABILITY_URL: &str = "https://store.steampowered.com/";
/// A reachability check must never hold the UI's offline decision hostage.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize)]
pub struct Reachability {
    pub online: bool,
    pub consecutive_failures: u32,
    pub last_failure_secs_ago: Option<u64>,
    pub tip: Option<String>,
}

/// Small in-process memory for the diagnostic fields returned to the UI.
/// It is updated only by explicit invocations; it never schedules another one.
#[derive(Default)]
pub struct ReachabilityState {
    consecutive_failures: u32,
    last_failure: Option<Instant>,
}

impl ReachabilityState {
    pub fn record_probe(&mut self, online: bool) -> Reachability {
        if online {
            self.consecutive_failures = 0;
            self.last_failure = None;
            return Reachability {
                online: true,
                consecutive_failures: 0,
                last_failure_secs_ago: None,
                tip: None,
            };
        }

        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.last_failure.is_none() {
            self.last_failure = Some(Instant::now());
        }
        Reachability {
            online: false,
            consecutive_failures: self.consecutive_failures,
            last_failure_secs_ago: self.last_failure.map(|at| at.elapsed().as_secs()),
            tip: Some("Steam est inaccessible. Cliquez pour réessayer.".to_string()),
        }
    }
}

/// Probe one Steam endpoint through the application's general Steam client.
/// The per-request timeout is intentional: `state.http` also serves other
/// Steam calls, so its timeout must not be changed just for this command.
pub async fn probe(http: &Client, url: &str) -> bool {
    http.get(url)
        // Match the store request convention in `steamstore.rs`: Steam rejects
        // the application's normal Origin/Referer defaults on some endpoints.
        .header(reqwest::header::ORIGIN, "https://store.steampowered.com")
        .header(reqwest::header::REFERER, "https://store.steampowered.com/")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

pub async fn probe_steam(http: &Client) -> bool {
    probe(http, STEAM_REACHABILITY_URL).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn local_server(response: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            socket.write_all(response).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        format!("http://{address}/")
    }

    #[tokio::test]
    async fn reachable_local_server_returns_online() {
        let url = local_server(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(probe(&Client::new(), &url).await);
    }

    #[tokio::test]
    async fn refused_local_port_returns_offline_promptly() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let started = Instant::now();
        assert!(!probe(&Client::new(), &format!("http://{address}/")).await);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "a refused local port must remain within the three-second request budget, took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn stalled_server_is_cut_off_by_the_explicit_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            std::future::pending::<()>().await;
        });

        let started = Instant::now();
        let online = tokio::time::timeout(
            Duration::from_secs(5),
            probe(&Client::new(), &format!("http://{address}/")),
        )
        .await
        .expect("the reachability timeout must complete before the outer test limit");
        assert!(!online);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "the three-second request timeout was not respected, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn state_counts_failures_and_resets_after_a_success() {
        let mut state = ReachabilityState::default();
        assert_eq!(state.record_probe(false).consecutive_failures, 1);
        assert_eq!(state.record_probe(false).consecutive_failures, 2);
        let recovered = state.record_probe(true);
        assert!(recovered.online);
        assert_eq!(recovered.consecutive_failures, 0);
        assert_eq!(recovered.last_failure_secs_ago, None);
    }

    #[test]
    fn state_keeps_the_start_of_a_consecutive_failure_series() {
        let mut state = ReachabilityState::default();
        state.record_probe(false);
        std::thread::sleep(Duration::from_millis(50));
        let second_failure = state.record_probe(false);

        let failure_age = state
            .last_failure
            .expect("a failed probe must record the failure-series start")
            .elapsed();
        assert!(
            failure_age >= Duration::from_millis(40),
            "the failure-series start was reset after {:?}",
            failure_age
        );
        assert_eq!(
            second_failure.last_failure_secs_ago,
            Some(failure_age.as_secs())
        );

        let recovered = state.record_probe(true);
        assert_eq!(recovered.last_failure_secs_ago, None);
    }
}
