//! Requests use a pinned storefront proxy; discovered config never chooses a host.
use crate::{Error, Result};
use reqwest::{Client, Method, RequestBuilder, Response};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use url::Url;

pub const ORIGIN: &str = "https://ametllerorigen.com";
pub const CALLBACK: &str = "https://www.ametllerorigen.com/callback";
pub const MAX_BODY: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub client_id: String,
    pub organization_id: String,
    pub site_id: String,
    pub short_code: String,
}
impl Config {
    pub fn parse(html: &str) -> Result<Self> {
        let document = Html::parse_document(html);
        let selector = Selector::parse("script#mobify-data").map_err(|_| Error::Response)?;
        let mut nodes = document.select(&selector);
        let node = nodes.next().ok_or(Error::Response)?;
        if nodes.next().is_some() {
            return Err(Error::Response);
        }
        let value: Value = serde_json::from_str(&node.inner_html()).map_err(|_| Error::Response)?;
        let config: Self = serde_json::from_value(
            value
                .pointer("/__CONFIG__/app/commerceAPI/parameters")
                .cloned()
                .ok_or(Error::Response)?,
        )
        .map_err(|_| Error::Response)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if self.organization_id != "f_ecom_blzv_prd"
            || self.site_id != "ametller"
            || self.short_code != "4jppt37a"
            || self.client_id.len() != 36
            || !self
                .client_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            return Err(Error::Input(
                "Unexpected storefront configuration. Review the integration before authenticating.",
            ));
        }
        Ok(())
    }
}

pub struct Api {
    pub(crate) client: Client,
    pub config: Config,
    base: String,
    pub locale: String,
}
impl Api {
    fn client() -> Result<Client> {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("ametller-mcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| Error::Network)
    }
    pub async fn discover(locale: &str) -> Result<Self> {
        validate_locale(locale)?;
        let client = Self::client()?;
        let response = send(client.get(format!("{ORIGIN}/{locale}/online"))).await?;
        check_status(&response)?;
        let body = bounded_body(response).await?;
        let config = Config::parse(std::str::from_utf8(&body).map_err(|_| Error::Response)?)?;
        Ok(Self {
            client,
            config,
            base: format!("{ORIGIN}/mobify/proxy/api"),
            locale: locale.into(),
        })
    }
    #[cfg(test)]
    pub(crate) fn mock(base: String) -> Self {
        Self {
            client: Self::client().unwrap(),
            config: Config {
                client_id: "fd3c9db8-2a0d-4f4b-9e74-294e068f9ae4".into(),
                organization_id: "f_ecom_blzv_prd".into(),
                site_id: "ametller".into(),
                short_code: "4jppt37a".into(),
            },
            base,
            locale: "es".into(),
        }
    }
    pub fn auth_url(&self, action: &str) -> String {
        format!(
            "{}/shopper/auth/v1/organizations/{}/oauth2/{action}",
            self.base, self.config.organization_id
        )
    }
    pub async fn request(
        &self,
        method: Method,
        service: &str,
        resource: &str,
        token: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value> {
        let url = format!(
            "{}/{service}/v1/organizations/{}/{resource}",
            self.base, self.config.organization_id
        );
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .query(&[("siteId", self.config.site_id.as_str())])
            .query(query);
        // The customer API rejects locale; custom calls do not declare it.
        // Only send it to services whose storefront SDK contract accepts it.
        if matches!(
            service,
            "product/shopper-products" | "search/shopper-search" | "checkout/shopper-baskets"
        ) {
            request = request.query(&[("locale", self.locale.as_str())]);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        json_response(send(request).await?).await
    }
}
pub fn validate_locale(locale: &str) -> Result<()> {
    if matches!(locale, "es" | "ca") {
        Ok(())
    } else {
        Err(Error::Input("Locale must be es or ca."))
    }
}
pub fn id(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 150
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Err(Error::Input(
            "Identifier must contain 1-150 ASCII letters, digits, underscores or hyphens.",
        ))
    } else {
        Ok(value)
    }
}
pub async fn send(request: RequestBuilder) -> Result<Response> {
    request.send().await.map_err(|_| Error::Network)
}
fn check_status(response: &Response) -> Result<()> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(Error::Http(response.status().as_u16()))
    }
}
pub async fn bounded_body(mut response: Response) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(Error::Response);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
        if chunk.len() > MAX_BODY.saturating_sub(body.len()) {
            return Err(Error::Response);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
pub async fn json_response(response: Response) -> Result<Value> {
    check_status(&response)?;
    let body = bounded_body(response).await?;
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).map_err(|_| Error::Response)
}
pub fn callback_code(location: &str, expected_state: &str) -> Result<String> {
    let url = Url::parse(location).map_err(|_| Error::Response)?;
    let expected = Url::parse(CALLBACK).map_err(|_| Error::Response)?;
    if url.origin() != expected.origin()
        || url.path() != expected.path()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::Input(
            "Unexpected login redirect; authentication stopped.",
        ));
    }
    // SLAS currently omits state from its direct HTTPS response. No browser
    // callback is accepted: the initiating request and PKCE verifier bind the
    // exchange. If state is echoed, still reject mismatch or duplication.
    let query: Vec<_> = url.query_pairs().collect();
    let codes: Vec<_> = query.iter().filter(|(k, _)| k == "code").collect();
    let states: Vec<_> = query.iter().filter(|(k, _)| k == "state").collect();
    if query.iter().any(|(k, _)| k == "error")
        || codes.len() != 1
        || codes[0].1.is_empty()
        || codes[0].1.len() > 8192
        || states.len() > 1
        || states
            .first()
            .is_some_and(|state| state.1 != expected_state)
    {
        return Err(Error::Input(
            "Login did not return a valid matching authorization response. Interactive challenges are not supported; check the official website.",
        ));
    }
    Ok(codes[0].1.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    #[tokio::test]
    async fn customer_and_custom_endpoints_reject_locale_but_keep_site_and_call_parameters() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        Mock::given(method("GET"))
            .respond_with(|request: &wiremock::Request| {
                let query: std::collections::HashMap<_, _> =
                    request.url.query_pairs().into_owned().collect();
                assert_eq!(query.get("siteId").map(String::as_str), Some("ametller"));
                // Reproduce the live customer API's rejection of unknown locale.
                if query.contains_key("locale") {
                    return ResponseTemplate::new(400);
                }
                if request.url.path().contains("/custom/") {
                    assert_eq!(query.get("c_productIds").map(String::as_str), Some("42171"));
                }
                ResponseTemplate::new(200).set_body_json(serde_json::json!({}))
            })
            .expect(3)
            .mount(&server)
            .await;
        for (service, resource, query) in [
            ("customer/shopper-customers", "customers/customer1", vec![]),
            (
                "customer/shopper-customers",
                "customers/customer1/baskets",
                vec![],
            ),
            (
                "custom/saved-cart",
                "sushi-reserve",
                vec![("c_productIds", "42171".into())],
            ),
        ] {
            api.request(Method::GET, service, resource, "synthetic", &query, None)
                .await
                .unwrap();
        }
    }
    #[tokio::test]
    async fn catalog_and_supported_basket_operations_keep_locale() {
        let server = MockServer::start().await;
        let mut api = Api::mock(server.uri());
        api.locale = "ca".into();
        Mock::given(wiremock::matchers::query_param("siteId", "ametller"))
            .and(wiremock::matchers::query_param("locale", "ca"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(7)
            .mount(&server)
            .await;
        for (method, service, resource) in [
            (Method::GET, "product/shopper-products", "products/42171"),
            (Method::GET, "search/shopper-search", "product-search"),
            (Method::GET, "checkout/shopper-baskets", "baskets/basket1"),
            (Method::POST, "checkout/shopper-baskets", "baskets"),
            (
                Method::POST,
                "checkout/shopper-baskets",
                "baskets/basket1/items",
            ),
            (
                Method::PATCH,
                "checkout/shopper-baskets",
                "baskets/basket1/items/line1",
            ),
            (
                Method::DELETE,
                "checkout/shopper-baskets",
                "baskets/basket1/items/line1",
            ),
        ] {
            api.request(method, service, resource, "synthetic", &[], None)
                .await
                .unwrap();
        }
    }
    #[test]
    fn callback_is_pinned_and_unambiguous() {
        assert_eq!(
            callback_code(&format!("{CALLBACK}?code=abc&state=xyz"), "xyz").unwrap(),
            "abc"
        );
        for bad in [
            "https://evil.example/callback?code=abc&state=xyz",
            "https://www.ametllerorigen.com/callback?code=a&code=b&state=xyz",
            "https://www.ametllerorigen.com/callback?code=abc&state=bad",
        ] {
            assert!(callback_code(bad, "xyz").is_err());
        }
    }
    #[test]
    fn slas_direct_callback_may_omit_state_but_configuration_is_pinned() {
        assert_eq!(
            callback_code(&format!("{CALLBACK}?code=abc&usid=opaque"), "request-state").unwrap(),
            "abc"
        );
        let config = Api::mock("http://127.0.0.1".into()).config;
        let html = format!(
            "<script id=\"mobify-data\">{}</script>",
            serde_json::json!({"__CONFIG__":{"app":{"commerceAPI":{"parameters":config}}}})
        );
        assert!(Config::parse(&html).is_ok());
        assert!(Config::parse(&html.replace("f_ecom_blzv_prd", "another_org")).is_err());
    }
    #[tokio::test]
    async fn response_size_and_redirects_are_bounded() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_BODY + 1]))
            .mount(&server)
            .await;
        assert!(
            bounded_body(
                send(Api::client().unwrap().get(server.uri()))
                    .await
                    .unwrap()
            )
            .await
            .is_err()
        );
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", "http://127.0.0.1:1/private"),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            send(Api::client().unwrap().get(server.uri()))
                .await
                .unwrap()
                .status(),
            302
        );
    }
}
