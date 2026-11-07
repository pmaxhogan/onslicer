//! The Onshape REST client: one shared reqwest client with redirects
//! disabled, manual redirect following that keeps auth only on onshape.com,
//! and the API call counter.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use reqwest::header::{ACCEPT, LOCATION};
use reqwest::{redirect, Response, StatusCode, Url};
use tauri::{AppHandle, Manager};

use crate::auth::Auth;
use crate::model::CallStats;
use crate::settings;

pub const BASE: &str = "https://cad.onshape.com";
pub const JSON_ACCEPT: &str = "application/json;charset=UTF-8;qs=0.09";
const MAX_HOPS: usize = 5;

/// The shared HTTP client. Redirects are followed by hand in [`Onshape::get`].
pub fn http_client() -> reqwest::Client {
    // reqwest is built without a bundled crypto provider (same as the updater
    // plugin); ring is the process default. Err only means one is installed.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .redirect(redirect::Policy::none())
        .user_agent(concat!("onslicer/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build the HTTP client")
}

/// `path` resolved against the Onshape base URL; absolute URLs pass through.
pub fn api_url(path: &str) -> Result<Url, String> {
    Url::parse(BASE)
        .and_then(|base| base.join(path))
        .map_err(|e| format!("bad Onshape URL {path}: {e}"))
}

/// Auth is only ever sent to Onshape itself over https.
fn is_onshape(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|h| h.ends_with(".onshape.com"))
}

/// An error with its source chain, which is where reqwest keeps the cause.
pub fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out
}

/// Managed state.
pub struct Onshape {
    client: reqwest::Client,
    session_calls: AtomicU64,
    /// Serializes read-modify-write of the persisted counter, so parallel
    /// requests (thumbnails) don't lose increments.
    count_lock: Mutex<()>,
}

impl Onshape {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            session_calls: AtomicU64::new(0),
            count_lock: Mutex::new(()),
        }
    }

    /// GET `start`. Onshape redirects some endpoints (e.g. STL export) to a
    /// sibling *.onshape.com host; an automatic redirect would drop the auth
    /// headers on that cross-origin hop, so redirects are followed here (at
    /// most 5) and auth is re-attached while the URL stays on onshape.com.
    /// On a 401/403 from Onshape, auth gets one chance to refresh its
    /// credentials, after which the request starts over once.
    pub async fn get(&self, app: &AppHandle, start: Url, accept: &str) -> Result<Response, String> {
        let auth = app.state::<Auth>();
        let mut url = start.clone();
        let mut hops = 0;
        let mut retried = false;
        while hops < MAX_HOPS {
            let onshape = is_onshape(&url);
            let mut req = self.client.get(url.clone()).header(ACCEPT, accept);
            if onshape {
                req = req.headers(auth.headers_for(app, &url).await?);
            }
            let res = req
                .send()
                .await
                .map_err(|e| format!("request failed: {}", error_chain(&e)))?;
            let status = res.status();
            if onshape && status.as_u16() < 400 {
                self.count_call(app);
            }
            if onshape
                && !retried
                && (status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN)
            {
                retried = true;
                if auth.handle_auth_failure(app).await? {
                    url = start.clone();
                    hops = 0;
                    continue;
                }
            }
            if status.is_redirection() {
                let loc = res
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| format!("redirect without location ({})", status.as_u16()))?;
                url = url
                    .join(loc)
                    .map_err(|e| format!("bad redirect location {loc}: {e}"))?;
                hops += 1;
                continue;
            }
            return Ok(res);
        }
        Err(format!("too many redirects: {start}"))
    }

    /// Our own estimate, to show roughly where the yearly allocation stands.
    /// Onshape's count at https://cad.onshape.com/user/developer is
    /// authoritative.
    fn count_call(&self, app: &AppHandle) {
        self.session_calls.fetch_add(1, Ordering::Relaxed);
        let _guard = self.count_lock.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(store) = settings::store(app) else {
            return;
        };
        let total = store
            .get(settings::API_CALLS_TOTAL)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        store.set(settings::API_CALLS_TOTAL, total + 1);
        let has_since = store
            .get(settings::API_CALLS_SINCE)
            .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()));
        if !has_since {
            store.set(settings::API_CALLS_SINCE, settings::now_iso());
        }
    }

    pub fn call_stats(&self, app: &AppHandle) -> CallStats {
        let _guard = self.count_lock.lock().unwrap_or_else(|e| e.into_inner());
        let store = settings::store(app).ok();
        let get = |key: &str| store.as_ref().and_then(|s| s.get(key));
        CallStats {
            session: self.session_calls.load(Ordering::Relaxed),
            total: get(settings::API_CALLS_TOTAL)
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            since: get(settings::API_CALLS_SINCE).and_then(|v| v.as_str().map(String::from)),
        }
    }

    pub fn session_calls(&self) -> u64 {
        self.session_calls.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_only_goes_to_onshape_over_https() {
        assert!(is_onshape(&api_url("/api/v6/documents").unwrap()));
        assert!(is_onshape(
            &Url::parse("https://us-east-1.onshape.com/x").unwrap()
        ));
        assert!(!is_onshape(
            &Url::parse("http://cad.onshape.com/x").unwrap()
        ));
        assert!(!is_onshape(
            &Url::parse("https://onshape.com.evil.io/x").unwrap()
        ));
        assert!(!is_onshape(
            &Url::parse("https://evilonshape.com/x").unwrap()
        ));
    }

    #[test]
    fn api_url_resolves_paths_and_keeps_absolute_urls() {
        assert_eq!(
            api_url("/api/v6/documents?limit=20").unwrap().as_str(),
            "https://cad.onshape.com/api/v6/documents?limit=20"
        );
        assert_eq!(
            api_url("https://cad.onshape.com/api/thumbnails/x?t=1")
                .unwrap()
                .as_str(),
            "https://cad.onshape.com/api/thumbnails/x?t=1"
        );
    }
}
