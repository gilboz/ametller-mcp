//! Private, temporary Chrome session used only to capture a registered SLAS response.
//! Never reads passwords, cookies, request bodies, or another browser profile.
use crate::{Error, Result, api::Config};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{net::TcpStream, time::Instant};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use zeroize::Zeroizing;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_MESSAGE: usize = 512 * 1024;
const MAX_TOKEN_BODY: usize = 128 * 1024;
const MAX_TRACKED: usize = 32;
const CLOSED: &str =
    "The login browser closed before sign-in completed. Run `ametller-mcp login` again.";
const PROTOCOL: &str = "Could not communicate with the private login browser. Close it and retry `ametller-mcp login`.";

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Open installed Chrome and return only a registered token response. The caller
/// must verify it with Ametller before persisting credentials.
pub async fn capture(locale: &str, config: &Config) -> Result<Zeroizing<String>> {
    crate::api::validate_locale(locale)?;
    config.validate()?;
    let mut browser = Browser::launch()?;
    let mut cdp = None;
    eprintln!(
        "Opening a private Chrome window. Sign in on Ametller's website; this window will close when your session is captured."
    );
    let outcome = tokio::select! {
        outcome = tokio::time::timeout(LOGIN_TIMEOUT, async {
            let endpoint = browser.endpoint().await?;
            let socket_config = WebSocketConfig::default()
                .max_message_size(Some(MAX_MESSAGE))
                .max_frame_size(Some(MAX_MESSAGE));
            let (socket, _) = connect_async_with_config(endpoint, Some(socket_config), true)
                .await.map_err(|_| Error::Input(PROTOCOL))?;
            cdp = Some(Cdp { socket, next_id: 0 });
            capture_session(cdp.as_mut().expect("assigned above"), &mut browser, locale, config).await
        }) => outcome.unwrap_or(Err(Error::Input("Browser login timed out after five minutes. Run `ametller-mcp login` and sign in again."))),
        _ = tokio::signal::ctrl_c() => Err(Error::Input("Browser login cancelled. No session was saved.")),
    };
    // Graceful shutdown closes this isolated browser's renderer children too.
    // The process guard still kills and waits if CDP setup or shutdown failed.
    if let Some(cdp) = cdp.as_mut() {
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            cdp.send("Browser.close", json!({}), None),
        )
        .await;
        let deadline = Instant::now() + Duration::from_secs(3);
        while browser.running().unwrap_or(false) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    browser.close();
    outcome
}

struct Browser {
    child: Child,
    // Kept until the child has been killed and reaped, including future cancellation.
    profile: TempDir,
}
impl Browser {
    fn launch() -> Result<Self> {
        let executable = executable().ok_or(Error::Input("Browser login requires installed Google Chrome or Chromium. Install Chrome, or use `ametller-mcp login --password` for terminal login."))?;
        let profile = tempfile::Builder::new()
            .prefix("ametller-login-")
            .tempdir()
            .map_err(|_| Error::Storage)?;
        let mut command = Command::new(executable);
        command
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .args([
                "--remote-debugging-address=127.0.0.1",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--disable-sync",
                "--disable-background-networking",
                "about:blank",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // A terminal Ctrl-C belongs to the CLI; it performs orderly cleanup.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command
            .spawn()
            .map_err(|_| Error::Input("Could not start installed Chrome for login."))?;
        Ok(Self { child, profile })
    }

    fn running(&mut self) -> Result<bool> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|_| Error::Input(PROTOCOL))
    }

    async fn endpoint(&mut self) -> Result<String> {
        let file = self.profile.path().join("DevToolsActivePort");
        loop {
            if !self.running()? {
                return Err(Error::Input(CLOSED));
            }
            if let Ok(file) = File::open(&file) {
                let mut text = String::new();
                file.take(1025)
                    .read_to_string(&mut text)
                    .map_err(|_| Error::Input(PROTOCOL))?;
                if text.len() > 1024 {
                    return Err(Error::Input(PROTOCOL));
                }
                // Chrome may be in the middle of writing its two-line file.
                if text.lines().count() >= 2 {
                    return debug_endpoint(&text);
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn close(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        self.close();
        // TempDir removes the profile after this method returns.
    }
}

fn executable() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ];
    #[cfg(not(target_os = "macos"))]
    let candidates = [
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/opt/google/chrome/chrome",
    ];
    candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn debug_endpoint(text: &str) -> Result<String> {
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 2 || !lines[0].bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Input(PROTOCOL));
    }
    let port: u16 = lines[0].parse().map_err(|_| Error::Input(PROTOCOL))?;
    let suffix = lines[1]
        .strip_prefix("/devtools/browser/")
        .ok_or(Error::Input(PROTOCOL))?;
    if port == 0
        || suffix.is_empty()
        || suffix.len() > 128
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(Error::Input(PROTOCOL));
    }
    Ok(format!("ws://127.0.0.1:{port}{}", lines[1]))
}

struct Cdp {
    socket: Socket,
    next_id: u64,
}
impl Cdp {
    async fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<u64> {
        self.next_id += 1;
        let mut message = json!({"id": self.next_id, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|_| Error::Input(PROTOCOL))?;
        Ok(self.next_id)
    }

    async fn receive(&mut self) -> Result<Value> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) if text.len() <= MAX_MESSAGE => {
                    return serde_json::from_str(&text).map_err(|_| Error::Input(PROTOCOL));
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => return Err(Error::Input(CLOSED)),
                _ => return Err(Error::Input(PROTOCOL)),
            }
        }
    }

    async fn call(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let id = self.send(method, params, session).await?;
        loop {
            let mut value = self.receive().await?;
            if value["id"].as_u64() == Some(id) {
                if value.get("error").is_some() {
                    return Err(Error::Input(PROTOCOL));
                }
                return Ok(value["result"].take());
            }
        }
    }
}

async fn capture_session(
    cdp: &mut Cdp,
    browser: &mut Browser,
    locale: &str,
    config: &Config,
) -> Result<Zeroizing<String>> {
    let created = cdp
        .call("Target.createTarget", json!({"url":"about:blank"}), None)
        .await?;
    let target = protocol_id(&created["targetId"])?;
    let attached = cdp
        .call(
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
            None,
        )
        .await?;
    let session = protocol_id(&attached["sessionId"])?;
    cdp.call("Network.enable", json!({"maxTotalBufferSize":4*1024*1024,"maxResourceBufferSize":MAX_TOKEN_BODY,"maxPostDataSize":0}), Some(&session)).await?;
    let navigate = cdp
        .send(
            "Page.navigate",
            json!({"url":format!("https://www.ametllerorigen.com/{locale}/login")}),
            Some(&session),
        )
        .await?;
    let mut state = CaptureState::default();
    let mut check_process = tokio::time::interval(Duration::from_millis(250));
    loop {
        let message = tokio::select! {
            message = cdp.receive() => message?,
            _ = check_process.tick() => {
                if !browser.running()? { return Err(Error::Input(CLOSED)); }
                continue;
            }
        };
        if message["method"] == "Target.detachedFromTarget"
            && message["params"]["sessionId"] == session
        {
            return Err(Error::Input(CLOSED));
        }
        if message["sessionId"] != session {
            continue;
        }
        if message["id"].as_u64() == Some(navigate)
            && (message.get("error").is_some() || message["result"].get("errorText").is_some())
        {
            return Err(Error::Input(
                "The Ametller login page could not load. Check your connection and retry.",
            ));
        }
        match state.handle(message, config)? {
            CaptureAction::Read(request_id) => {
                let id = cdp
                    .send(
                        "Network.getResponseBody",
                        json!({"requestId":request_id}),
                        Some(&session),
                    )
                    .await?;
                state.pending.insert(id, request_id);
            }
            CaptureAction::Captured(body) => return Ok(body),
            CaptureAction::None => {}
        }
    }
}

fn protocol_id(value: &Value) -> Result<String> {
    value
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
        .map(str::to_owned)
        .ok_or(Error::Input(PROTOCOL))
}

fn token_url(url: &str, config: &Config) -> bool {
    // Exact raw URL equality also rejects explicit ports, credentials, escaped
    // path segments, fragments, queries, and URL-parser path normalization.
    let path = format!(
        "/shopper/auth/v1/organizations/{}/oauth2/token",
        config.organization_id
    );
    url == format!("https://ametllerorigen.com/mobify/proxy/api{path}")
        || url == format!("https://www.ametllerorigen.com/mobify/proxy/api{path}")
        || url == format!("https://4jppt37a.api.commercecloud.salesforce.com{path}")
}

#[derive(Default)]
struct CaptureState {
    responses: HashSet<String>,
    pending: HashMap<u64, String>,
}
enum CaptureAction {
    None,
    Read(String),
    Captured(Zeroizing<String>),
}
impl CaptureState {
    fn handle(&mut self, mut value: Value, config: &Config) -> Result<CaptureAction> {
        if let Some(id) = value["id"].as_u64() {
            if self.pending.remove(&id).is_some()
                && value.get("error").is_none()
                && let Some(body) = response_body(value["result"].take())
                && crate::auth::is_registered_response(&body, config)
            {
                return Ok(CaptureAction::Captured(body));
            }
            return Ok(CaptureAction::None);
        }
        let params = &value["params"];
        match value["method"].as_str() {
            Some("Network.responseReceived") => {
                if params["response"]["status"]
                    .as_u64()
                    .is_some_and(|status| (200..300).contains(&status))
                    && params["response"]["url"]
                        .as_str()
                        .is_some_and(|url| token_url(url, config))
                {
                    let id = protocol_id(&params["requestId"])?;
                    if self.responses.len() + self.pending.len() >= MAX_TRACKED {
                        return Err(Error::Input(
                            "Too many pending authentication responses. Close the login browser and retry.",
                        ));
                    }
                    self.responses.insert(id);
                }
            }
            Some("Network.loadingFinished") => {
                if let Some(id) = params["requestId"].as_str()
                    && self.responses.remove(id)
                {
                    return Ok(CaptureAction::Read(id.to_string()));
                }
            }
            Some("Network.loadingFailed") => {
                if let Some(id) = params["requestId"].as_str() {
                    self.responses.remove(id);
                }
            }
            _ => {}
        }
        Ok(CaptureAction::None)
    }
}

fn response_body(mut result: Value) -> Option<Zeroizing<String>> {
    let encoded = result["base64Encoded"].as_bool()?;
    let Value::String(body) = result["body"].take() else {
        return None;
    };
    let body = Zeroizing::new(body);
    if body.len() > MAX_TOKEN_BODY {
        return None;
    }
    if encoded {
        let decoded = Zeroizing::new(STANDARD.decode(body.as_bytes()).ok()?);
        Some(Zeroizing::new(
            std::str::from_utf8(&decoded).ok()?.to_owned(),
        ))
    } else {
        Some(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Api;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn config() -> Config {
        Api::mock("http://127.0.0.1".into()).config
    }
    fn token_response(guest: bool) -> String {
        let identity = if guest {
            "gcid:guest::chid:ametller::upn:Guest"
        } else {
            "rcid:customer::chid:ametller::upn:registered"
        };
        let token = format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD
                .encode(json!({"isb":identity,"exp":crate::auth::now()+3600}).to_string())
        );
        json!({"access_token":token,"refresh_token":"synthetic-refresh","expires_in":3600})
            .to_string()
    }
    fn response(id: &str) -> Value {
        json!({"method":"Network.responseReceived","params":{"requestId":id,"response":{"url":"https://www.ametllerorigen.com/mobify/proxy/api/shopper/auth/v1/organizations/f_ecom_blzv_prd/oauth2/token","status":200}}})
    }

    #[test]
    fn authentication_urls_are_exactly_pinned() {
        let config = config();
        let path = "/shopper/auth/v1/organizations/f_ecom_blzv_prd/oauth2/token";
        let good = format!("https://www.ametllerorigen.com/mobify/proxy/api{path}");
        for url in [
            &good,
            &good.replace("www.", ""),
            &format!("https://4jppt37a.api.commercecloud.salesforce.com{path}"),
        ] {
            assert!(token_url(url, &config));
        }
        for bad in [
            good.replace("https:", "http:"),
            good.replace(".com/", ".com:443/"),
            good.replace("https://", "https://user@"),
            good.replace(".com/", ".com.evil/"),
            format!("{good}?query=true"),
            format!("{good}#fragment"),
            good.replace("f_ecom_blzv_prd", "another_org"),
            good.replace("/oauth2/", "/x/../oauth2/"),
            good.replace("/oauth2/", "/%6fauth2/"),
            good.replace("/token", "/token/"),
        ] {
            assert!(!token_url(&bad, &config), "{bad}");
        }
    }

    #[test]
    fn devtools_file_can_only_select_loopback_websocket() {
        assert_eq!(
            debug_endpoint("41234\n/devtools/browser/test-id\n").unwrap(),
            "ws://127.0.0.1:41234/devtools/browser/test-id"
        );
        for bad in [
            "0\n/devtools/browser/a",
            "65536\n/devtools/browser/a",
            "80\n//evil/a",
            "80\n/devtools/browser/../a",
            "80\n/devtools/browser/a?host=evil",
            "80\n/devtools/browser/a\nextra",
            "80\n/devtools/browser/",
            "+80\n/devtools/browser/a",
        ] {
            assert!(debug_endpoint(bad).is_err());
        }
    }

    #[test]
    fn waits_for_finished_then_ignores_guest_and_malformed_bodies() {
        let mut state = CaptureState::default();
        let config = config();
        assert!(matches!(
            state
                .handle(
                    json!({"method":"Network.loadingFinished","params":{"requestId":"unknown"}}),
                    &config
                )
                .unwrap(),
            CaptureAction::None
        ));
        for (index, body) in [
            token_response(true),
            "malformed".into(),
            token_response(false),
        ]
        .into_iter()
        .enumerate()
        {
            let request = format!("request{index}");
            assert!(matches!(
                state.handle(response(&request), &config).unwrap(),
                CaptureAction::None
            ));
            assert!(
                matches!(state.handle(json!({"method":"Network.loadingFinished","params":{"requestId":request}}), &config).unwrap(), CaptureAction::Read(id) if id == request)
            );
            let command = index as u64 + 1;
            state.pending.insert(command, request);
            let result = state
                .handle(
                    json!({"id":command,"result":{"body":body,"base64Encoded":false}}),
                    &config,
                )
                .unwrap();
            assert_eq!(matches!(result, CaptureAction::Captured(_)), index == 2);
        }
        assert!(state.responses.is_empty() && state.pending.is_empty());
    }

    #[test]
    fn failed_requests_and_body_errors_do_not_accumulate() {
        let config = config();
        let mut state = CaptureState::default();
        state.handle(response("failed"), &config).unwrap();
        state
            .handle(
                json!({"method":"Network.loadingFailed","params":{"requestId":"failed"}}),
                &config,
            )
            .unwrap();
        assert!(state.responses.is_empty());
        state.pending.insert(1, "body".into());
        assert!(matches!(
            state
                .handle(
                    json!({"id":1,"error":{"message":"a private error"}}),
                    &config
                )
                .unwrap(),
            CaptureAction::None
        ));
        assert!(state.pending.is_empty());
        for index in 0..MAX_TRACKED {
            state
                .handle(response(&format!("request{index}")), &config)
                .unwrap();
        }
        assert!(state.handle(response("overflow"), &config).is_err());
        assert_eq!(state.responses.len(), MAX_TRACKED);
    }

    #[test]
    fn token_body_decoding_is_bounded_and_handles_base64() {
        let body = token_response(false);
        assert_eq!(
            *response_body(json!({"body":STANDARD.encode(&body),"base64Encoded":true})).unwrap(),
            body
        );
        assert!(
            response_body(json!({"body":"x".repeat(MAX_TOKEN_BODY+1),"base64Encoded":false}))
                .is_none()
        );
        assert!(response_body(json!({"body":"!!","base64Encoded":true})).is_none());
        assert!(
            response_body(json!({"body":STANDARD.encode([0xff]),"base64Encoded":true})).is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_guard_reaps_before_deleting_profile() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().to_owned();
        let child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let mut browser = Browser { child, profile };
        browser.close();
        assert!(!browser.running().unwrap());
        assert!(path.exists());
        drop(browser);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn websocket_rejects_oversized_frames_without_exposing_payload() {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = ws
                .send(Message::Text(
                    "sensitive".repeat(MAX_MESSAGE / 9 + 1).into(),
                ))
                .await;
        });
        let (socket, _) = connect_async_with_config(
            format!("ws://{address}"),
            Some(
                WebSocketConfig::default()
                    .max_message_size(Some(MAX_MESSAGE))
                    .max_frame_size(Some(MAX_MESSAGE)),
            ),
            true,
        )
        .await
        .unwrap();
        let mut cdp = Cdp { socket, next_id: 0 };
        let error = cdp.receive().await.unwrap_err();
        assert_eq!(error.to_string(), PROTOCOL);
        server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_cdp_captures_registered_response_after_interleaved_guest() {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            for (method, result) in [
                ("Target.createTarget", json!({"targetId":"target"})),
                ("Target.attachToTarget", json!({"sessionId":"session"})),
                ("Network.enable", json!({})),
            ] {
                let command: Value =
                    serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(command["method"], method);
                ws.send(Message::Text(
                    json!({"id":command["id"],"result":result})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            }
            let navigate: Value =
                serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(navigate["method"], "Page.navigate");
            assert_eq!(
                navigate["params"]["url"],
                "https://www.ametllerorigen.com/ca/login"
            );
            // Emit token events before the navigation acknowledgement. No token
            // body may be requested until its loadingFinished event is seen.
            for request in ["guest", "registered"] {
                let mut event = response(request);
                event["sessionId"] = json!("session");
                ws.send(Message::Text(event.to_string().into()))
                    .await
                    .unwrap();
            }
            ws.send(Message::Text(
                json!({"id":navigate["id"],"sessionId":"session","result":{}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            for request in ["guest", "registered"] {
                ws.send(Message::Text(json!({"method":"Network.loadingFinished","sessionId":"session","params":{"requestId":request}}).to_string().into())).await.unwrap();
                let command: Value =
                    serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(command["method"], "Network.getResponseBody");
                assert_eq!(command["params"]["requestId"], request);
                ws.send(Message::Text(json!({"id":command["id"],"sessionId":"session","result":{"body":STANDARD.encode(token_response(request == "guest")),"base64Encoded":true}}).to_string().into())).await.unwrap();
            }
        });
        let (socket, _) = connect_async_with_config(format!("ws://{address}"), None, true)
            .await
            .unwrap();
        let mut cdp = Cdp { socket, next_id: 0 };
        let mut browser = Browser {
            child: Command::new("/bin/sleep").arg("30").spawn().unwrap(),
            profile: tempfile::tempdir().unwrap(),
        };
        let body = tokio::time::timeout(
            Duration::from_secs(5),
            capture_session(&mut cdp, &mut browser, "ca", &config()),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(crate::auth::is_registered_response(&body, &config()));
        server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_future_drops_process_guard_and_private_profile() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().to_owned();
        let browser = Browser {
            child: Command::new("/bin/sleep").arg("30").spawn().unwrap(),
            profile,
        };
        let result = tokio::time::timeout(Duration::from_millis(10), async move {
            let _browser = browser;
            std::future::pending::<()>().await;
        })
        .await;
        assert!(result.is_err());
        assert!(!path.exists());
    }

    /// Deliberate local smoke test only: opens installed Chrome at about:blank,
    /// verifies the real CDP connection, and closes its isolated process/profile.
    #[tokio::test]
    #[ignore = "opens installed Chrome at about:blank; run explicitly on a desktop"]
    async fn installed_chrome_blank_launch_and_cleanup() {
        let mut browser = Browser::launch().unwrap();
        let path = browser.profile.path().to_owned();
        tokio::time::timeout(Duration::from_secs(20), async {
            let endpoint = browser.endpoint().await.unwrap();
            let (socket, _) = connect_async_with_config(endpoint, None, true)
                .await
                .unwrap();
            let mut cdp = Cdp { socket, next_id: 0 };
            let version = cdp
                .call("Browser.getVersion", json!({}), None)
                .await
                .unwrap();
            assert!(version["product"].as_str().is_some());
            cdp.send("Browser.close", json!({}), None).await.unwrap();
            while browser.running().unwrap() {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        drop(browser);
        assert!(!path.exists());
    }
}
