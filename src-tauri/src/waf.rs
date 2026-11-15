//! AWS WAF in front of Onshape challenges the export endpoints (part studio
//! STL, assembly translations) for every client that is not a browser: the
//! request comes back as an empty 202 with `x-amzn-waf-action: challenge` (a
//! silent JavaScript check) or `captcha`, whatever the user agent or session.
//! API-key requests skip it, but those are metered and can run out (402).
//!
//! A browser that passes the check gets an `aws-waf-token` cookie for the
//! host, and the token works from any client. So on a challenge a webview
//! window opens on a challenged URL of that host, the check runs in it (shown
//! to the user only when it does not finish on its own within a few seconds,
//! or right away for a CAPTCHA), and its token is read from the webview's
//! cookie store. The token lives in memory only; it is short-lived.

use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, SET_COOKIE};
use reqwest::{Response, StatusCode, Url};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const TOKEN_COOKIE: &str = "aws-waf-token";
const LABEL: &str = "onshape-check";
/// A silent challenge usually finishes in a second or two in the hidden
/// window; past this the window is shown so the user can see what is going on.
const SHOW_AFTER: Duration = Duration::from_secs(3);
const TIMEOUT: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_millis(250);
/// IDs of nothing, for a URL the WAF challenges without touching any data:
/// behind the check it answers 401 or 404.
const PROBE_ID: &str = "0123456789abcdef01234567";

/// What the WAF wants, when `res` is a WAF challenge rather than a real answer.
pub fn action(res: &Response) -> Option<String> {
    if res.status() != StatusCode::ACCEPTED {
        return None;
    }
    res.headers()
        .get("x-amzn-waf-action")
        .map(|v| v.to_str().unwrap_or("").to_ascii_lowercase())
}

/// A refreshed token the WAF set on a response, if any.
pub fn token_from_response(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next()?.trim().split_once('='))
        .find(|(k, v)| *k == TOKEN_COOKIE && !v.is_empty())
        .map(|(_, v)| v.to_owned())
}

/// The URL the check runs on: an export URL on the challenged host. GET, so
/// the page can load it, and pointing at no document.
pub fn probe_url(challenged: &Url) -> Result<Url, String> {
    let host = challenged
        .host_str()
        .ok_or("the challenged URL has no host")?;
    Url::parse(&format!(
        "https://{host}/api/v6/partstudios/d/{PROBE_ID}/w/{PROBE_ID}/e/{PROBE_ID}/stl"
    ))
    .map_err(|e| format!("bad security check URL: {e}"))
}

/// Runs the check in a webview and returns the new token. `stale` is the
/// token the WAF just turned down; the webview's cookie store may still hold
/// it, so it does not count as an answer.
pub async fn solve(
    app: &AppHandle,
    challenged: &Url,
    captcha: bool,
    stale: Option<&str>,
) -> Result<String, String> {
    let url = probe_url(challenged)?;
    if let Some(old) = app.get_webview_window(LABEL) {
        let _ = old.destroy();
    }
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url.clone()))
        .title("onslicer - Onshape security check")
        .inner_size(520.0, 680.0)
        .center()
        .visible(captcha)
        .build()
        .map_err(|e| format!("could not open the Onshape security check: {e}"))?;
    let started = Instant::now();
    let result = loop {
        tokio::time::sleep(POLL).await;
        if app.get_webview_window(LABEL).is_none() {
            break Err(
                "the Onshape security check was closed before it finished - try again".into(),
            );
        }
        // Reading cookies must not happen on the main thread (WebView2
        // deadlock); this runs on the async runtime.
        if let Ok(cookies) = window.cookies_for_url(url.clone()) {
            let token = cookies
                .iter()
                .find(|c| c.name() == TOKEN_COOKIE && !c.value().is_empty())
                .map(|c| c.value().to_owned())
                .filter(|t| Some(t.as_str()) != stale);
            if let Some(token) = token {
                break Ok(token);
            }
        }
        let elapsed = started.elapsed();
        if elapsed > SHOW_AFTER && !window.is_visible().unwrap_or(true) {
            let _ = window.show();
            let _ = window.set_focus();
        }
        if elapsed > TIMEOUT {
            break Err("the Onshape security check timed out - try again".into());
        }
    };
    let _ = window.destroy();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn probe_url_keeps_the_host_and_points_at_no_document() {
        let u = probe_url(&Url::parse("https://cad.onshape.com/api/v6/translations/x").unwrap())
            .unwrap();
        assert_eq!(
            u.as_str(),
            format!(
                "https://cad.onshape.com/api/v6/partstudios/d/{PROBE_ID}/w/{PROBE_ID}/e/{PROBE_ID}/stl"
            )
        );
    }

    #[test]
    fn token_from_response_reads_only_the_waf_cookie() {
        let mut h = HeaderMap::new();
        for v in [
            "on=abc; Path=/",
            "aws-waf-token=t0k=en; Domain=.onshape.com; Secure",
        ] {
            h.append(SET_COOKIE, HeaderValue::from_str(v).unwrap());
        }
        assert_eq!(token_from_response(&h).as_deref(), Some("t0k=en"));
        let mut h = HeaderMap::new();
        h.append(
            SET_COOKIE,
            HeaderValue::from_static("aws-waf-token=; Max-Age=0"),
        );
        assert_eq!(token_from_response(&h), None);
    }
}
