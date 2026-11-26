//! CLI-only credentials: password is never accepted by MCP, arguments, or env.
use crate::{
    Error, Result,
    api::{Api, CALLBACK, callback_code, id, json_response, send},
    store::Store,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub customer_id: String,
    pub expires_at: u64,
    #[zeroize(skip)]
    pub config: crate::api::Config,
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn nonce() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

fn token_identity(token: &str) -> Result<(String, String, bool, u64)> {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 || token.len() > 32768 {
        return Err(Error::Response);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| Error::Response)?;
    let data: Value = serde_json::from_slice(&payload).map_err(|_| Error::Response)?;
    let segments: Vec<_> = data["isb"]
        .as_str()
        .ok_or(Error::Response)?
        .split("::")
        .collect();
    let guest = segments.contains(&"upn:Guest");
    let one = |prefix: &str| -> Result<String> {
        let values: Vec<_> = segments
            .iter()
            .filter_map(|part| part.strip_prefix(prefix))
            .collect();
        if values.len() != 1 {
            return Err(Error::Response);
        }
        Ok(id(values[0])?.to_string())
    };
    Ok((
        one(if guest { "gcid:" } else { "rcid:" })?,
        one("chid:")?,
        guest,
        data["exp"].as_u64().ok_or(Error::Response)?,
    ))
}
fn registered_session(value: &Value, api: &Api, previous: Option<&Session>) -> Result<Session> {
    let access = value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(Error::Response)?;
    let (customer, site, guest, expiry) = token_identity(access)?;
    if guest
        || site != api.config.site_id
        || previous.is_some_and(|old| old.customer_id != customer)
    {
        return Err(Error::Input(
            "The authentication response does not match the registered Ametller account.",
        ));
    }
    let refresh = value["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| previous.map(|s| s.refresh_token.as_str()))
        .ok_or(Error::Response)?;
    if refresh.len() > 32768 || expiry <= now() {
        return Err(Error::Response);
    }
    let expires_in = value["expires_in"]
        .as_u64()
        .filter(|seconds| *seconds > 0 && *seconds <= 31 * 86400)
        .ok_or(Error::Response)?;
    Ok(Session {
        access_token: access.into(),
        refresh_token: refresh.into(),
        customer_id: customer,
        expires_at: expiry.min(now().saturating_add(expires_in)),
        config: api.config.clone(),
    })
}
async fn exchange(api: &Api, code: &str, verifier: &str) -> Result<Value> {
    json_response(
        send(api.client.post(api.auth_url("token")).form(&[
            ("grant_type", "authorization_code_pkce"),
            ("code", code),
            ("code_verifier", verifier),
            ("client_id", api.config.client_id.as_str()),
            ("channel_id", api.config.site_id.as_str()),
            ("redirect_uri", CALLBACK),
        ]))
        .await?,
    )
    .await
}
async fn authorization_code(response: reqwest::Response, state: &str) -> Result<String> {
    let status = response.status().as_u16();
    if !matches!(status, 302 | 303) {
        let reason = match status {
            400 => {
                "The login request was rejected; this does not establish that the password is wrong."
            }
            401 => {
                "Authentication was not accepted. Verify your credentials on the official website."
            }
            403 => "The service denied this sign-in request.",
            404 => "The authentication route is unavailable.",
            429 => "The sign-in service is rate limiting requests. Wait before trying again.",
            500..=599 => "The authentication service failed. Try again later.",
            _ => "An unexpected login response was received.",
        };
        // Never echo an authentication body, redirect URL or server-supplied text.
        return Err(Error::LoginHttp { status, reason });
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(Error::Response)?;
    callback_code(location, state)
}
pub async fn guest(api: &Api) -> Result<String> {
    let verifier = nonce();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = nonce();
    let response = send(api.client.get(api.auth_url("authorize")).query(&[
        ("hint", "guest"),
        ("response_type", "code"),
        ("client_id", api.config.client_id.as_str()),
        ("channel_id", api.config.site_id.as_str()),
        ("code_challenge", challenge.as_str()),
        ("redirect_uri", CALLBACK),
        ("state", state.as_str()),
    ]))
    .await?;
    let code = authorization_code(response, &state).await?;
    let value = exchange(api, &code, &verifier).await?;
    let token = value["access_token"].as_str().ok_or(Error::Response)?;
    let (_, site, is_guest, expiry) = token_identity(token)?;
    if !is_guest || site != api.config.site_id || expiry <= now() {
        return Err(Error::Response);
    }
    Ok(token.into())
}
pub async fn login(api: &Api, email: &str, password: &str, store: &Store) -> Result<Value> {
    if email.is_empty()
        || email.len() > 254
        || email.contains([':', '\r', '\n'])
        || password.is_empty()
        || password.len() > 4096
    {
        return Err(Error::Input(
            "Email or password has an invalid length or format.",
        ));
    }
    let _lock = store.lock().await?;
    let verifier = nonce();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = nonce();
    let usid = uuid::Uuid::new_v4().to_string();
    let response = send(
        api.client
            .post(api.auth_url("login"))
            .basic_auth(email, Some(password))
            .form(&[
                ("client_id", api.config.client_id.as_str()),
                ("channel_id", api.config.site_id.as_str()),
                ("code_challenge", challenge.as_str()),
                ("redirect_uri", CALLBACK),
                ("usid", usid.as_str()),
                ("state", state.as_str()),
            ]),
    )
    .await?;
    let code = authorization_code(response, &state).await?;
    let tokens = exchange(api, &code, &verifier).await?;
    let session = registered_session(&tokens, api, None)?;
    let profile = validate_customer(api, &session).await?;
    store.save(&session)?;
    Ok(profile)
}

/// Caller holds the store lock until their complete account operation finishes.
pub async fn registered(api: &Api, store: &Store) -> Result<Session> {
    let mut session = store.load()?.ok_or(Error::LoginRequired)?;
    if session.config != api.config {
        return Err(Error::SessionChanged);
    }
    let (customer, site, guest, _) = token_identity(&session.access_token)?;
    if guest || customer != session.customer_id || site != api.config.site_id {
        return Err(Error::SessionChanged);
    }
    if session.expires_at <= now() + 60 {
        let tokens = json_response(
            send(api.client.post(api.auth_url("token")).form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", session.refresh_token.as_str()),
                ("client_id", api.config.client_id.as_str()),
                ("channel_id", api.config.site_id.as_str()),
            ]))
            .await?,
        )
        .await?;
        session = registered_session(&tokens, api, Some(&session))?;
        // Persist replacement refresh credentials before any fallible follow-up.
        // Otherwise a transient customer read would strand a rotated credential.
        store.save(&session)?;
    }
    validate_customer(api, &session).await?;
    Ok(session)
}
pub async fn validate_customer(api: &Api, session: &Session) -> Result<Value> {
    let result = api
        .request(
            Method::GET,
            "customer/shopper-customers",
            &format!("customers/{}", id(&session.customer_id)?),
            &session.access_token,
            &[],
            None,
        )
        .await
        .map_err(|error| match error {
            Error::Http(status) => Error::AccountHttp(status),
            other => other,
        })?;
    if result["customerId"].as_str() != Some(&session.customer_id)
        || result["authType"].as_str() != Some("registered")
    {
        return Err(Error::Input(
            "The server did not confirm the registered account. Sign in again.",
        ));
    }
    Ok(
        json!({"authenticated":true,"verification":"live","customer_id":session.customer_id,"expires_at":session.expires_at,"mode":"registered"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::CredentialStore;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path, query_param, query_param_is_missing},
    };
    pub(crate) fn token(customer: &str, expiry: u64) -> String {
        format!("e30.{}.signature",URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"isb":format!("rcid:{customer}::chid:ametller::upn:registered"),"exp":expiry})).unwrap()))
    }
    #[tokio::test]
    async fn interrupted_account_switch_requires_login_before_account_requests() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: CredentialStore::File,
        };
        let _lock = store.lock().await.unwrap();
        let original = Session {
            access_token: token("accountA", now() + 3600),
            refresh_token: "original-synthetic".into(),
            customer_id: "accountA".into(),
            expires_at: now() + 3600,
            config: api.config.clone(),
        };
        store.save(&original).unwrap();
        let replacement = Session {
            access_token: token("accountB", now() + 3600),
            refresh_token: "replacement-synthetic".into(),
            customer_id: "accountB".into(),
            expires_at: now() + 3600,
            config: api.config.clone(),
        };
        // Reproduce a save interrupted after replacing the private credential.
        std::fs::write(
            store.directory.join("credentials.json"),
            serde_json::to_vec(&replacement).unwrap(),
        )
        .unwrap();
        let result = registered(&api, &store).await;
        assert!(
            matches!(result, Err(Error::Input(message)) if message.contains("ametller-mcp login"))
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        let cached = crate::service::App::new(store.clone(), "ca".into())
            .unwrap()
            .status(false)
            .await
            .unwrap();
        assert_eq!(cached["customer_id"], "accountA");
        assert_eq!(cached["verification"], "not_checked");
        assert!(cached["authenticated"].is_null());
        assert!(server.received_requests().await.unwrap().is_empty());

        // Missing metadata cannot expose an orphaned credential, either.
        std::fs::remove_file(store.directory.join("connected.json")).unwrap();
        assert!(matches!(
            registered(&api, &store).await,
            Err(Error::LoginRequired)
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
        // The successful-login save path establishes the new identity binding.
        store.save(&replacement).unwrap();
        Mock::given(method("GET"))
            .and(path(
                "/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/accountB",
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"customerId":"accountB","authType":"registered"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            registered(&api, &store).await.unwrap().customer_id,
            "accountB"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn refresh_rotation_persisted_and_reused_without_repeating() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: CredentialStore::File,
        };
        let _lock = store.lock().await.unwrap();
        let old = Session {
            access_token: token("customer1", now() + 3600),
            refresh_token: "old-secret".into(),
            customer_id: "customer1".into(),
            expires_at: now() - 1,
            config: api.config.clone(),
        };
        store.save(&old).unwrap();
        Mock::given(method("POST")).and(path("/shopper/auth/v1/organizations/f_ecom_blzv_prd/oauth2/token")).and(body_string_contains("refresh_token=old-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":token("customer1",now()+3600),"refresh_token":"replacement-secret","expires_in":3600}))).expect(1).mount(&server).await;
        Mock::given(method("GET"))
            .and(path(
                "/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/customer1",
            ))
            .and(query_param("siteId", "ametller"))
            .and(query_param_is_missing("locale"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"customerId":"customer1","authType":"registered"})),
            )
            .expect(2)
            .mount(&server)
            .await;
        assert_eq!(
            registered(&api, &store).await.unwrap().refresh_token,
            "replacement-secret"
        );
        assert_eq!(
            registered(&api, &store).await.unwrap().refresh_token,
            "replacement-secret"
        );
    }
    #[test]
    fn token_identity_rejects_guest_or_account_switch_on_refresh() {
        let api = Api::mock("http://127.0.0.1".into());
        let old = Session {
            access_token: token("a", now() + 3000),
            refresh_token: "secret".into(),
            customer_id: "a".into(),
            expires_at: now() + 3000,
            config: api.config.clone(),
        };
        let response = json!({"access_token":token("b",now()+3000),"refresh_token":"rotated","expires_in":3000});
        assert!(registered_session(&response, &api, Some(&old)).is_err());
    }
    #[tokio::test]
    async fn direct_login_exchanges_code_and_validates_customer_before_saving() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: CredentialStore::File,
        };
        Mock::given(method("POST"))
            .and(path(
                "/shopper/auth/v1/organizations/f_ecom_blzv_prd/oauth2/login",
            ))
            .and(wiremock::matchers::header(
                "Authorization",
                "Basic dGVzdEBleGFtcGxlLmNvbTpwYXNzd29yZA==",
            ))
            .respond_with(|request: &wiremock::Request| {
                let pairs: std::collections::HashMap<_, _> =
                    url::form_urlencoded::parse(&request.body)
                        .into_owned()
                        .collect();
                assert!(pairs["code_challenge"].len() >= 43);
                let usid = uuid::Uuid::parse_str(&pairs["usid"]).unwrap();
                assert_eq!(usid.get_version_num(), 4);
                assert_eq!(usid.to_string(), pairs["usid"]);
                ResponseTemplate::new(303).insert_header(
                    "Location",
                    format!("{CALLBACK}?code=synthetic-code&state={}", pairs["state"]),
                )
            })
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/shopper/auth/v1/organizations/f_ecom_blzv_prd/oauth2/token"))
            .and(body_string_contains("code=synthetic-code"))
            .and(body_string_contains("grant_type=authorization_code_pkce"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":token("customer1",now()+3600),"refresh_token":"fake-refresh","expires_in":3600})))
            .expect(1).mount(&server).await;
        Mock::given(method("GET"))
            .and(path(
                "/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/customer1",
            ))
            .and(query_param("siteId", "ametller"))
            .and(query_param_is_missing("locale"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"customerId":"customer1","authType":"registered"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            login(&api, "test@example.com", "password", &store)
                .await
                .unwrap()["verification"],
            "live"
        );
        assert_eq!(store.load().unwrap().unwrap().refresh_token, "fake-refresh");
    }

    #[tokio::test]
    async fn login_http_failures_report_status_without_echoing_secrets_or_replacing_session() {
        for status in [200, 400, 401, 403, 404, 429, 503] {
            let server = MockServer::start().await;
            let api = Api::mock(server.uri());
            let temp = tempfile::tempdir().unwrap();
            let store = Store {
                directory: temp.path().join("state"),
                backend: CredentialStore::File,
            };
            {
                let _guard = store.lock().await.unwrap();
                store
                    .save(&Session {
                        access_token: token("old-account", now() + 3600),
                        refresh_token: "old-refresh".into(),
                        customer_id: "old-account".into(),
                        expires_at: now() + 3600,
                        config: api.config.clone(),
                    })
                    .unwrap();
            }
            Mock::given(method("POST"))
                .and(path(
                    "/shopper/auth/v1/organizations/f_ecom_blzv_prd/oauth2/login",
                ))
                .respond_with(
                    ResponseTemplate::new(status).set_body_string(
                        "private-password private-token private-email@example.com",
                    ),
                )
                .expect(1)
                .mount(&server)
                .await;
            let error = login(&api, "new@example.com", "private-password", &store)
                .await
                .unwrap_err();
            assert!(matches!(error, Error::LoginHttp { status: actual, .. } if actual == status));
            let message = error.to_string();
            assert!(message.contains(&status.to_string()));
            assert!(
                !message.contains("private-password")
                    && !message.contains("private-token")
                    && !message.contains("private-email")
            );
            assert!(!message.contains("CAPTCHA"));
            assert_eq!(store.load().unwrap().unwrap().customer_id, "old-account");
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn failed_post_login_account_check_preserves_existing_session_and_pending_cart() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: CredentialStore::File,
        };
        {
            let _guard = store.lock().await.unwrap();
            store
                .save(&Session {
                    access_token: token("old-account", now() + 3600),
                    refresh_token: "old-refresh".into(),
                    customer_id: "old-account".into(),
                    expires_at: now() + 3600,
                    config: api.config.clone(),
                })
                .unwrap();
            store
                .set_pending(&json!({"customer_id":"old-account","stage":"set_item"}))
                .unwrap();
        }
        Mock::given(method("POST"))
            .and(path(
                "/shopper/auth/v1/organizations/f_ecom_blzv_prd/oauth2/login",
            ))
            .respond_with(
                ResponseTemplate::new(303)
                    .insert_header("Location", format!("{CALLBACK}?code=synthetic-code")),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/shopper/auth/v1/organizations/f_ecom_blzv_prd/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":token("new-account", now()+3600),"refresh_token":"new-refresh","expires_in":3600})))
            .expect(1).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/new-account"))
            .and(query_param("siteId", "ametller"))
            .and(query_param_is_missing("locale"))
            .respond_with(ResponseTemplate::new(400).set_body_string("private-account-information"))
            .expect(1).mount(&server).await;
        let error = login(&api, "new@example.com", "synthetic-password", &store)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::AccountHttp(400)));
        assert!(error.to_string().contains("Account verification"));
        assert!(!error.to_string().contains("private-account-information"));
        let saved = store.load().unwrap().unwrap();
        assert_eq!(saved.customer_id, "old-account");
        assert_eq!(saved.refresh_token, "old-refresh");
        assert_eq!(
            store.pending().unwrap().unwrap()["customer_id"],
            "old-account"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }
}
