use crate::{
    Error, Result,
    api::{Api, id, validate_locale},
    auth, cart,
    store::Store,
};
use reqwest::Method;
use scraper::Html;
use serde_json::{Value, json};
use url::Url;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct App {
    pub store: Store,
    pub locale: String,
}
impl App {
    pub fn new(store: Store, locale: String) -> Result<Self> {
        validate_locale(&locale)?;
        Ok(Self { store, locale })
    }
    pub async fn status(&self, live: bool) -> Result<Value> {
        if !live {
            let metadata = if self.store.has_session()? {
                self.store.metadata()?
            } else {
                None
            };
            return Ok(match metadata {
                Some(metadata) => {
                    json!({"authenticated":null,"connected":true,"mode":"registered_cached","verification":"not_checked","customer_id":metadata["customer_id"],"expires_at":metadata["expires_at"],"credential_store":metadata["credential_store"],"instructions":"Use authentication_status with live=true to verify access."})
                }
                None => {
                    json!({"authenticated":false,"connected":false,"mode":"login_required","verification":"not_checked","instructions":"Catalog search is available without login. Run ametller-mcp login privately for your account cart."})
                }
            });
        }
        let _lock = self.store.lock().await?;
        if !self.store.has_session()? {
            return Err(Error::LoginRequired);
        }
        let api = Api::discover(&self.locale).await?;
        let session = auth::registered(&api, &self.store).await?;
        Ok(
            json!({"authenticated":true,"connected":true,"mode":"registered","verification":"live","customer_id":session.customer_id,"expires_at":session.expires_at}),
        )
    }
    pub async fn login(&self, email: &str, password: &str) -> Result<Value> {
        auth::login(
            &Api::discover(&self.locale).await?,
            email,
            password,
            &self.store,
        )
        .await
    }
    pub async fn login_browser(&self) -> Result<Value> {
        let api = Api::discover(&self.locale).await?;
        let response = crate::browser::capture(&self.locale, &api.config).await?;
        auth::accept_browser_session(&api, &response, &self.store).await
    }
    pub async fn logout(&self) -> Result<Value> {
        let _lock = self.store.lock().await?;
        self.store.delete()?;
        Ok(
            json!({"authenticated":false,"local_credentials_removed":true,"remote_session_revoked":false}),
        )
    }
    pub async fn search(&self, query: &str, limit: u32, offset: u32) -> Result<Value> {
        if query.trim().is_empty()
            || query.chars().count() > 200
            || !(1..=50).contains(&limit)
            || offset > 10000
        {
            return Err(Error::Input(
                "Search requires 1-200 characters, limit 1-50, and offset 0-10000.",
            ));
        }
        let api = Api::discover(&self.locale).await?;
        let token = Zeroizing::new(auth::guest(&api).await?);
        let result = api
            .request(
                Method::GET,
                "search/shopper-search",
                "product-search",
                &token,
                &[
                    ("q", query.into()),
                    ("limit", limit.to_string()),
                    ("offset", offset.to_string()),
                    ("expand", "images,prices".into()),
                ],
                None,
            )
            .await?;
        let hits = result["hits"].as_array().ok_or(Error::Response)?;
        if hits.len() > limit as usize {
            return Err(Error::Response);
        }
        Ok(
            json!({"query":query,"locale":self.locale,"pricing":"guest_catalog_estimate","total":result["total"],"limit":limit,"offset":offset,"products":hits.iter().map(|hit|product_output(hit,"guest")).collect::<Vec<_>>() }),
        )
    }
    pub async fn product(&self, product: &str) -> Result<Value> {
        id(product)?;
        let api = Api::discover(&self.locale).await?;
        let token = Zeroizing::new(auth::guest(&api).await?);
        let detail = api
            .request(
                Method::GET,
                "product/shopper-products",
                &format!("products/{product}"),
                &token,
                &[
                    ("expand", "images,prices,availability,promotions".into()),
                    ("allImages", "true".into()),
                ],
                None,
            )
            .await?;
        if detail["id"].as_str() != Some(product) {
            return Err(Error::Response);
        }
        Ok(product_output(&detail, "guest"))
    }
    pub async fn cart(&self, clear_uncertain: bool) -> Result<Value> {
        let _lock = self.store.lock().await?;
        if !self.store.has_session()? {
            return Err(Error::LoginRequired);
        }
        let api = Api::discover(&self.locale).await?;
        let session = auth::registered(&api, &self.store).await?;
        cart::get(&api, &session, &self.store, clear_uncertain).await
    }
    pub async fn set_cart_item(
        &self,
        product: &str,
        quantity: &str,
        item: Option<&str>,
    ) -> Result<Value> {
        id(product)?;
        if let Some(item) = item {
            id(item)?;
        }
        let desired = cart::quantity(quantity)?;
        let _lock = self.store.lock().await?;
        if !self.store.has_session()? {
            return Err(Error::LoginRequired);
        }
        let api = Api::discover(&self.locale).await?;
        let session = auth::registered(&api, &self.store).await?;
        cart::set(&api, &session, &self.store, product, desired, item).await
    }
}
fn official_url(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    let url = Url::parse(text).ok()?;
    if url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some("ametllerorigen.com" | "www.ametllerorigen.com")
        )
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
    {
        Some(text.into())
    } else {
        None
    }
}
fn text(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    let html = Html::parse_fragment(text);
    Some(
        html.root_element()
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}
pub fn product_output(value: &Value, context: &str) -> Value {
    let mut photos = Vec::new();
    let mut add = |image: &Value| {
        if let Some(url) =
            official_url(&image["link"]).or_else(|| official_url(&image["disBaseLink"]))
            && !photos.contains(&url)
            && photos.len() < 12
        {
            photos.push(url);
        }
    };
    add(&value["image"]);
    if let Some(groups) = value["imageGroups"].as_array() {
        for group in groups {
            if let Some(images) = group["images"].as_array() {
                for image in images {
                    add(image);
                }
            }
        }
    }
    json!({"product_id":value.get("id").or_else(||value.get("productId")),"name":value.get("name").or_else(||value.get("productName")),"photo_urls":photos,
        "description":text(&value["shortDescription"]),"long_description":text(&value["longDescription"]),"product_url":official_url(&value["slugUrl"]),
        "currency":value["currency"],"price":value["price"],"price_per_unit":value["pricePerUnit"],"unit_measure":value["unitMeasure"],"unit_quantity":value["unitQuantity"],
        "minimum_quantity":value["minOrderQuantity"],"quantity_step":value["stepQuantity"],"approximate_weight":value["c_ao_pesoAproximado"],
        "reservation_availability":value["c_ao_disponibilidad"],"reservation_date":value["c_ao_fechaReserva"],
        "orderable":value.pointer("/inventory/orderable").and_then(Value::as_bool),"pricing_context":context,"final_total_known":false})
}
