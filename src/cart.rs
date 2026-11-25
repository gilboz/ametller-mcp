//! Cart operations are absolute, serialized and never automatically replayed.
use crate::{
    Error, Result,
    api::{Api, id},
    auth::Session,
    store::Store,
};
use reqwest::Method;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::str::FromStr;

pub fn quantity(value: &str) -> Result<Decimal> {
    if value.is_empty()
        || value.len() > 32
        || !value.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return Err(Error::Input(
            "Quantity must be a nonnegative decimal string, for example 2 or 0.5.",
        ));
    }
    let value =
        Decimal::from_str_exact(value).map_err(|_| Error::Input("Invalid decimal quantity."))?;
    if value > Decimal::from(10000) || value.scale() > 6 {
        return Err(Error::Input(
            "Quantity must be at most 10000 with at most 6 decimal places.",
        ));
    }
    Ok(value)
}
fn decimal(value: &Value) -> Result<Decimal> {
    match value {
        Value::Number(n) => Decimal::from_str(&n.to_string()).map_err(|_| Error::Response),
        _ => Err(Error::Response),
    }
}
pub fn validate_quantity(product: &Value, desired: Decimal) -> Result<()> {
    if desired.is_zero() {
        return Ok(());
    }
    if product
        .pointer("/inventory/orderable")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(Error::Input(
            "Product availability is unavailable or unknown; inspect the official product page.",
        ));
    }
    if reservation_product(product)
        || product.pointer("/type/master").and_then(Value::as_bool) == Some(true)
        || product.pointer("/type/bundle").and_then(Value::as_bool) == Some(true)
        || product.pointer("/type/set").and_then(Value::as_bool) == Some(true)
        || nonempty(&product["options"])
    {
        return Err(Error::Input(
            "This product requires options, a bundle choice, or reservation; edit it on the official website.",
        ));
    }
    let minimum = decimal(&product["minOrderQuantity"])?;
    let step = decimal(&product["stepQuantity"])?;
    if minimum <= Decimal::ZERO || step <= Decimal::ZERO {
        return Err(Error::Response);
    }
    if desired < minimum
        || !(desired - minimum)
            .checked_rem(step)
            .ok_or(Error::Response)?
            .is_zero()
    {
        return Err(Error::Input(
            "Quantity does not match the product's minimum and purchase increment. Read get_product for the exact rules.",
        ));
    }
    Ok(())
}
fn nonempty(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::String(v) => !v.is_empty(),
        _ => true,
    }
}
fn reservation_product(product: &Value) -> bool {
    product
        .pointer("/inventory/preorderable")
        .and_then(Value::as_bool)
        == Some(true)
        || product["c_ao_disponibilidad"] == "05"
        || (product["c_ao_disponibilidad"] == "03" && nonempty(&product["c_ao_fechaReserva"]))
}
fn reservation_line(line: &Value) -> bool {
    [line, &line["detailedInfo"], &line["representedProduct"]]
        .into_iter()
        .any(reservation_product)
}
fn special_line(line: &Value) -> bool {
    nonempty(&line["optionItems"])
        || nonempty(&line["bonusDiscountLineItemId"])
        || line["bonusProductLineItem"] == true
        || reservation_line(line)
}
fn lines(basket: &Value) -> Result<&[Value]> {
    match basket.get("productItems") {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        _ => Err(Error::Response),
    }
}
pub fn select_line<'a>(
    basket: &'a Value,
    product: &str,
    item: Option<&str>,
) -> Result<Option<&'a Value>> {
    let matches: Vec<_> = lines(basket)?
        .iter()
        .filter(|line| line["productId"].as_str() == Some(product))
        .collect();
    let selected = if let Some(item) = item {
        let selected: Vec<_> = matches
            .iter()
            .copied()
            .filter(|line| line["itemId"].as_str() == Some(item))
            .collect();
        if selected.len() != 1 {
            return Err(Error::Input(
                "The selected item ID does not identify exactly one line of this product. Read get_cart again.",
            ));
        }
        Some(selected[0])
    } else {
        if matches.len() > 1 {
            return Err(Error::Input(
                "This product has multiple cart lines. Read get_cart and supply the exact item_id.",
            ));
        }
        matches.first().copied()
    };
    if selected.is_some_and(special_line) {
        return Err(Error::Input(
            "Options, bonus, or reservation cart lines must be edited on the official website.",
        ));
    }
    Ok(selected)
}
async fn baskets(api: &Api, session: &Session) -> Result<Vec<Value>> {
    let response = api
        .request(
            Method::GET,
            "customer/shopper-customers",
            &format!("customers/{}/baskets", id(&session.customer_id)?),
            &session.access_token,
            &[],
            None,
        )
        .await?;
    let baskets = match response.get("baskets") {
        Some(Value::Array(values)) => values.clone(),
        None if response["total"] == 0 => Vec::new(),
        _ => return Err(Error::Response),
    };
    if response
        .get("total")
        .is_some_and(|total| total.as_u64() != Some(baskets.len() as u64))
    {
        return Err(Error::Input(
            "Basket list is incomplete. Inspect your baskets on the official website.",
        ));
    }
    Ok(baskets)
}
async fn read_basket(api: &Api, session: &Session) -> Result<Option<Value>> {
    let all = baskets(api, session).await?;
    if all.len() > 1 {
        return Err(Error::Input(
            "Multiple baskets exist. Choose and reconcile them on Ametller's website before changing the cart.",
        ));
    }
    let Some(summary) = all.first() else {
        return Ok(None);
    };
    let basket_id = id(summary["basketId"].as_str().ok_or(Error::Response)?)?;
    let basket = api
        .request(
            Method::GET,
            "checkout/shopper-baskets",
            &format!("baskets/{basket_id}"),
            &session.access_token,
            &[],
            None,
        )
        .await?;
    if basket["basketId"].as_str() != Some(basket_id) {
        return Err(Error::Response);
    }
    Ok(Some(basket))
}
fn summary(basket: Option<&Value>, pending: Option<Value>) -> Result<Value> {
    let mut result = json!({"exists":basket.is_some(),"pricing":"account_cart_estimate","final_total_known":false,"checkout_url":"https://ametllerorigen.com/es/cart","pending_write":pending,"items":[]});
    if let Some(basket) = basket {
        let items: Vec<_> = lines(basket)?
            .iter()
            .map(|item| {
                let line_total = ["priceAfterOrderDiscount", "priceAfterItemDiscount", "price"]
                    .iter()
                    .find_map(|field| item.get(field).filter(|value| !value.is_null()));
                json!({
                    "item_id":item["itemId"],"product_id":item["productId"],
                    "name":item["productName"],"quantity":item["quantity"],
                    "base_unit_price":item["basePrice"],"line_total":line_total,
                    "currency":basket["currency"],"requires_website":special_line(item)
                })
            })
            .collect();
        result["items"] = json!(items);
        result["basket_id"] = basket["basketId"].clone();
        result["currency"] = basket["currency"].clone();
        result["product_total"] = basket["productTotal"].clone();
        result["shipping_total"] = basket["shippingTotal"].clone();
        result["tax_total"] = basket["taxTotal"].clone();
        result["order_total"] = basket["orderTotal"].clone();
    }
    Ok(result)
}
fn pending_matches(basket: Option<&Value>, pending: &Value, session: &Session) -> bool {
    if pending["customer_id"].as_str() != Some(&session.customer_id)
        || pending["stage"] != "set_item"
    {
        return false;
    }
    let Some(product) = pending["product_id"].as_str() else {
        return false;
    };
    let Some(desired) = pending["quantity"].as_str().and_then(|s| quantity(s).ok()) else {
        return false;
    };
    let item = pending["item_id"].as_str();
    let Some(basket) = basket else {
        return desired.is_zero() && item.is_none();
    };
    if !pending["basket_id"].is_null() && pending["basket_id"] != basket["basketId"] {
        return false;
    }
    match select_line(basket, product, item) {
        Ok(Some(line)) => decimal(&line["quantity"]).is_ok_and(|actual| actual == desired),
        Ok(None) => desired.is_zero(),
        // DELETE removes an explicit line ID; confirm its absence separately.
        Err(_) if desired.is_zero() && item.is_some() => lines(basket)
            .is_ok_and(|lines| !lines.iter().any(|line| line["itemId"].as_str() == item)),
        _ => false,
    }
}
pub async fn get(
    api: &Api,
    session: &Session,
    store: &Store,
    clear_uncertain: bool,
) -> Result<Value> {
    let mut pending = store.pending()?;
    if clear_uncertain
        && pending.as_ref().is_some_and(|pending| {
            pending["customer_id"].as_str() != Some(session.customer_id.as_str())
        })
    {
        return Err(Error::Input(
            "The unresolved cart write belongs to another account. Sign in to the account that owns it before using `cart --clear-uncertain`.",
        ));
    }
    let basket = read_basket(api, session).await?;
    if pending
        .as_ref()
        .is_some_and(|p| pending_matches(basket.as_ref(), p, session))
        || clear_uncertain
    {
        store.clear_pending()?;
        pending = None;
    }
    format_cart(api, session, basket.as_ref(), pending).await
}
async fn format_cart(
    api: &Api,
    session: &Session,
    basket: Option<&Value>,
    pending: Option<Value>,
) -> Result<Value> {
    let mut output = summary(basket, pending)?;
    output["checkout_url"] = json!(format!("https://ametllerorigen.com/{}/cart", api.locale));
    // Cart data itself is authoritative. Images are optional enrichment; a
    // failed catalog request must not hide the actual account cart.
    if let Some(items) = output["items"].as_array_mut() {
        let ids: Vec<_> = items
            .iter()
            .filter_map(|item| item["product_id"].as_str())
            .filter(|value| id(value).is_ok())
            .take(24)
            .collect();
        if !ids.is_empty()
            && let Ok(details) = api
                .request(
                    Method::GET,
                    "product/shopper-products",
                    "products",
                    &session.access_token,
                    &[
                        ("ids", ids.join(",")),
                        ("expand", "images,prices,availability".into()),
                    ],
                    None,
                )
                .await
            && let Some(products) = details["data"].as_array()
        {
            for item in items {
                if let Some(detail) = products
                    .iter()
                    .find(|detail| detail["id"] == item["product_id"])
                {
                    let product = crate::service::product_output(detail, "account");
                    item["photo_urls"] = product["photo_urls"].clone();
                    item["product_url"] = product["product_url"].clone();
                    item["approximate_weight"] = product["approximate_weight"].clone();
                    item["unit_measure"] = product["unit_measure"].clone();
                    item["reservation_availability"] = detail["c_ao_disponibilidad"].clone();
                    item["reservation_date"] = detail["c_ao_fechaReserva"].clone();
                    if reservation_product(detail) {
                        item["requires_website"] = json!(true);
                    }
                }
            }
        }
    }
    Ok(output)
}
async fn ensure_not_reserved(api: &Api, session: &Session, products: &[&str]) -> Result<()> {
    let result = api
        .request(
            Method::GET,
            "custom/saved-cart",
            "sushi-reserve",
            &session.access_token,
            &[("c_productIds", products.join(","))],
            None,
        )
        .await?;
    let reserved = result["reserveProductIds"]
        .as_array()
        .ok_or(Error::Response)?;
    if reserved.iter().any(|value| value.as_str().is_none()) {
        return Err(Error::Response);
    }
    if reserved
        .iter()
        .filter_map(Value::as_str)
        .any(|product| products.contains(&product))
    {
        return Err(Error::Input(
            "This product requires a reservation or delivery-date choice. Use the official website.",
        ));
    }
    Ok(())
}
async fn ensure_basket_not_reserved(
    api: &Api,
    session: &Session,
    basket: Option<&Value>,
    selected_product: &str,
) -> Result<()> {
    let Some(basket) = basket else {
        return Ok(());
    };
    let mut products = Vec::new();
    for line in lines(basket)? {
        if reservation_line(line) {
            return Err(Error::Input(
                "The cart contains reservation products. Edit this cart on the official website.",
            ));
        }
        let product = id(line["productId"].as_str().ok_or(Error::Response)?)?;
        if product != selected_product && !products.contains(&product) {
            products.push(product);
        }
    }
    // The storefront checks the catalog metadata of every existing product
    // before mixing normal and reservation items. Basket lines may omit it.
    for ids in products.chunks(24) {
        let response = api
            .request(
                Method::GET,
                "product/shopper-products",
                "products",
                &session.access_token,
                &[("ids", ids.join(",")), ("expand", "availability".into())],
                None,
            )
            .await?;
        let details = response["data"].as_array().ok_or(Error::Response)?;
        for product in ids {
            let mut matches = details.iter().filter(|detail| detail["id"] == *product);
            let detail = matches.next().ok_or(Error::Response)?;
            if matches.next().is_some() {
                return Err(Error::Response);
            }
            if reservation_product(detail) {
                return Err(Error::Input(
                    "The cart contains reservation products. Edit this cart on the official website.",
                ));
            }
        }
        ensure_not_reserved(api, session, ids).await?;
    }
    Ok(())
}
pub async fn set(
    api: &Api,
    session: &Session,
    store: &Store,
    product: &str,
    desired: Decimal,
    item: Option<&str>,
) -> Result<Value> {
    id(product)?;
    if let Some(item) = item {
        id(item)?;
    }
    if store.pending()?.is_some() {
        return Err(Error::Pending);
    }
    let mut basket = read_basket(api, session).await?;
    let line = if let Some(basket) = &basket {
        select_line(basket, product, item)?
    } else {
        if item.is_some() {
            return Err(Error::Input("No basket exists for the supplied item_id."));
        }
        None
    };
    if line.is_none() && desired.is_zero() {
        return format_cart(api, session, basket.as_ref(), None).await;
    }
    if line.is_some_and(|line| decimal(&line["quantity"]).is_ok_and(|old| old == desired)) {
        return format_cart(api, session, basket.as_ref(), None).await;
    }
    // Removing an ordinary line cannot introduce reservation mixing, and its
    // catalog record may no longer exist. select_line checked the target itself.
    if !desired.is_zero() {
        let detail = api
            .request(
                Method::GET,
                "product/shopper-products",
                &format!("products/{product}"),
                &session.access_token,
                &[("expand", "images,prices,availability".into())],
                None,
            )
            .await?;
        if detail["id"].as_str() != Some(product) {
            return Err(Error::Response);
        }
        if reservation_product(&detail) {
            return Err(Error::Input(
                "This product requires a reservation or delivery-date choice. Use the official website.",
            ));
        }
        validate_quantity(&detail, desired)?;
        ensure_not_reserved(api, session, &[product]).await?;
        ensure_basket_not_reserved(api, session, basket.as_ref(), product).await?;
    }
    let line_id = line
        .and_then(|line| line["itemId"].as_str())
        .map(str::to_owned);
    if line.is_some() && line_id.is_none() {
        return Err(Error::Response);
    }
    let mut pending = json!({"customer_id":session.customer_id,"product_id":product,"item_id":line_id,"quantity":desired.to_string(),"basket_id":basket.as_ref().map(|b|b["basketId"].clone()),"stage":"create_basket"});
    if basket.is_none() {
        store.set_pending(&pending)?;
        let created = api
            .request(
                Method::POST,
                "checkout/shopper-baskets",
                "baskets",
                &session.access_token,
                &[],
                Some(json!({})),
            )
            .await
            .map_err(|_| Error::Uncertain)?;
        let basket_id = created["basketId"].as_str().ok_or(Error::Uncertain)?;
        id(basket_id).map_err(|_| Error::Uncertain)?;
        pending["basket_id"] = json!(basket_id);
        basket = Some(created);
    }
    let basket_id = id(basket
        .as_ref()
        .and_then(|b| b["basketId"].as_str())
        .ok_or(Error::Response)?)?;
    pending["stage"] = json!("set_item");
    store.set_pending(&pending)?;
    let quantity = Value::Number(
        serde_json::Number::from_str(&desired.to_string()).map_err(|_| Error::Response)?,
    );
    let (method, resource, body) = if let Some(line_id) = line_id {
        id(&line_id)?;
        if desired.is_zero() {
            (
                Method::DELETE,
                format!("baskets/{basket_id}/items/{line_id}"),
                None,
            )
        } else {
            (
                Method::PATCH,
                format!("baskets/{basket_id}/items/{line_id}"),
                Some(json!({"quantity":quantity})),
            )
        }
    } else {
        (
            Method::POST,
            format!("baskets/{basket_id}/items"),
            Some(json!([{"productId":product,"quantity":quantity}])),
        )
    };
    api.request(
        method,
        "checkout/shopper-baskets",
        &resource,
        &session.access_token,
        &[],
        body,
    )
    .await
    .map_err(|_| Error::Uncertain)?;
    let actual = read_basket(api, session)
        .await
        .map_err(|_| Error::Uncertain)?;
    if !pending_matches(actual.as_ref(), &pending, session) {
        return Err(Error::Uncertain);
    }
    store.clear_pending()?;
    format_cart(api, session, actual.as_ref(), None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };
    #[test]
    fn exact_quantity_and_ambiguous_lines() {
        let product =
            json!({"inventory":{"orderable":true},"minOrderQuantity":0.1,"stepQuantity":0.1});
        assert!(validate_quantity(&product, quantity("0.3").unwrap()).is_ok());
        assert!(validate_quantity(&product, quantity("0.35").unwrap()).is_err());
        assert!(quantity("-1").is_err());
        assert!(quantity("NaN").is_err());
        let basket = json!({"productItems":[{"productId":"a","itemId":"one"},{"productId":"a","itemId":"two"}]});
        assert!(select_line(&basket, "a", None).is_err());
        assert!(select_line(&basket, "a", Some("two")).is_ok());
        assert!(
            select_line(
                &json!({"productItems":[{"productId":"a","itemId":"one","optionItems":[{}]}]}),
                "a",
                None
            )
            .is_err()
        );
    }
    #[test]
    fn line_totals_prefer_order_discounts_and_preserve_unknown_prices() {
        let basket: Value = serde_json::from_str(
            r#"{"productItems":[
                {"basePrice":3.41,"price":6.82,"priceAfterItemDiscount":6.32,"priceAfterOrderDiscount":5.82},
                {"price":6.82,"priceAfterItemDiscount":6.32,"priceAfterOrderDiscount":null},
                {"price":6.82,"priceAfterItemDiscount":null},
                {"price":6.82,"priceAfterItemDiscount":6.32,"priceAfterOrderDiscount":0},
                {}
            ]}"#,
        )
        .unwrap();
        let output = summary(Some(&basket), None).unwrap();
        let totals: Vec<_> = output["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["line_total"].to_string())
            .collect();
        assert_eq!(totals, ["5.82", "6.32", "6.82", "0", "null"]);
        assert_eq!(output["items"][0]["base_unit_price"].to_string(), "3.41");
        assert!(output["items"][0].get("unit_price").is_none());
        assert!(output["items"][4]["base_unit_price"].is_null());
    }
    #[tokio::test]
    async fn absent_cart_read_does_not_create() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        Mock::given(method("GET")).and(path("/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/customer1/baskets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total":0,"baskets":[]}))).expect(1).mount(&server).await;
        let session = Session {
            access_token: "fake".into(),
            refresh_token: "fake".into(),
            customer_id: "customer1".into(),
            expires_at: u64::MAX,
            config: api.config.clone(),
        };
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: crate::store::CredentialStore::File,
        };
        let _lock = store.lock().await.unwrap();
        assert_eq!(
            get(&api, &session, &store, false).await.unwrap()["exists"],
            false
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    async fn mock_existing_cart(server: &MockServer, basket: Value) {
        Mock::given(method("GET")).and(path("/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/customer1/baskets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total":1,"baskets":[{"basketId":"basket1"}]}))).mount(server).await;
        Mock::given(method("GET"))
            .and(path(
                "/checkout/shopper-baskets/v1/organizations/f_ecom_blzv_prd/baskets/basket1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(basket))
            .mount(server)
            .await;
        Mock::given(method("GET")).and(path("/product/shopper-products/v1/organizations/f_ecom_blzv_prd/products/42171"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"42171","inventory":{"orderable":true},"minOrderQuantity":1,"stepQuantity":1}))).mount(server).await;
        Mock::given(method("GET"))
            .and(path(
                "/custom/saved-cart/v1/organizations/f_ecom_blzv_prd/sushi-reserve",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"reserveProductIds":[]})))
            .mount(server)
            .await;
    }
    fn session(api: &Api) -> Session {
        Session {
            access_token: "synthetic".into(),
            refresh_token: "synthetic".into(),
            customer_id: "customer1".into(),
            expires_at: u64::MAX,
            config: api.config.clone(),
        }
    }
    #[tokio::test]
    async fn clearing_uncertain_cart_requires_the_pending_account() {
        for clear_uncertain in [false, true] {
            let server = MockServer::start().await;
            let api = Api::mock(server.uri());
            Mock::given(method("GET"))
                .and(path("/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/customer1/baskets"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total":0,"baskets":[]})))
                .expect(if clear_uncertain { 0 } else { 1 })
                .mount(&server).await;
            let temp = tempfile::tempdir().unwrap();
            let store = Store {
                directory: temp.path().join("state"),
                backend: crate::store::CredentialStore::File,
            };
            let _lock = store.lock().await.unwrap();
            let pending = json!({"customer_id":"another-account","product_id":"42171","quantity":"0","stage":"set_item"});
            store.set_pending(&pending).unwrap();
            let result = get(&api, &session(&api), &store, clear_uncertain).await;
            if clear_uncertain {
                assert!(
                    matches!(result, Err(Error::Input(message)) if message.contains("Sign in to the account that owns it"))
                );
                assert!(server.received_requests().await.unwrap().is_empty());
            } else {
                assert_eq!(result.unwrap()["pending_write"], pending);
            }
            assert_eq!(store.pending().unwrap(), Some(pending));
        }
    }
    #[tokio::test]
    async fn pending_account_can_clear_manually_or_by_observed_state() {
        for clear_uncertain in [false, true] {
            let server = MockServer::start().await;
            let api = Api::mock(server.uri());
            Mock::given(method("GET"))
                .and(path("/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/customer1/baskets"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total":0,"baskets":[]})))
                .expect(1).mount(&server).await;
            let temp = tempfile::tempdir().unwrap();
            let store = Store {
                directory: temp.path().join("state"),
                backend: crate::store::CredentialStore::File,
            };
            let _lock = store.lock().await.unwrap();
            store.set_pending(&json!({"customer_id":"customer1","product_id":"42171","quantity":if clear_uncertain { "1" } else { "0" },"stage":"set_item"})).unwrap();
            let result = get(&api, &session(&api), &store, clear_uncertain)
                .await
                .unwrap();
            assert!(result["pending_write"].is_null());
            assert!(store.pending().unwrap().is_none());
        }
    }
    #[tokio::test]
    async fn selected_dated_and_exclusive_reservations_never_write() {
        for (availability, date) in [("03", json!("2026-12-01")), ("05", Value::Null)] {
            let server = MockServer::start().await;
            let api = Api::mock(server.uri());
            mock_existing_cart(&server, json!({"basketId":"basket1","productItems":[]})).await;
            Mock::given(method("GET"))
                .and(path("/product/shopper-products/v1/organizations/f_ecom_blzv_prd/products/42171"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id":"42171","inventory":{"orderable":true},"minOrderQuantity":1,"stepQuantity":1,
                    "c_ao_disponibilidad":availability,"c_ao_fechaReserva":date
                })))
                .with_priority(1).expect(1).mount(&server).await;
            let temp = tempfile::tempdir().unwrap();
            let store = Store {
                directory: temp.path().join("state"),
                backend: crate::store::CredentialStore::File,
            };
            let _lock = store.lock().await.unwrap();
            assert!(matches!(
                set(&api, &session(&api), &store, "42171", Decimal::ONE, None).await,
                Err(Error::Input(_))
            ));
            assert!(store.pending().unwrap().is_none());
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method == "GET")
            );
        }
    }
    #[tokio::test]
    async fn existing_reservations_block_adding_normal_products_without_writes() {
        for (availability, date) in [
            ("03", json!("2026-12-01")),
            ("05", Value::Null),
            ("02", Value::Null),
        ] {
            let server = MockServer::start().await;
            let api = Api::mock(server.uri());
            mock_existing_cart(
                &server,
                json!({"basketId":"basket1","productItems":[
                    {"productId":"reserved","itemId":"reservation1","quantity":1}
                ]}),
            )
            .await;
            Mock::given(method("GET"))
                .and(path(
                    "/product/shopper-products/v1/organizations/f_ecom_blzv_prd/products",
                ))
                .and(query_param("ids", "reserved"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{
                    "id":"reserved","c_ao_disponibilidad":availability,"c_ao_fechaReserva":date
                }]})))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(
                    "/custom/saved-cart/v1/organizations/f_ecom_blzv_prd/sushi-reserve",
                ))
                .and(query_param("c_productIds", "reserved"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"reserveProductIds":["reserved"]})),
                )
                .with_priority(1)
                .mount(&server)
                .await;
            let temp = tempfile::tempdir().unwrap();
            let store = Store {
                directory: temp.path().join("state"),
                backend: crate::store::CredentialStore::File,
            };
            let _lock = store.lock().await.unwrap();
            assert!(matches!(
                set(&api, &session(&api), &store, "42171", Decimal::ONE, None).await,
                Err(Error::Input(_))
            ));
            assert!(store.pending().unwrap().is_none());
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method == "GET")
            );
        }
    }
    #[tokio::test]
    async fn deletion_skips_catalog_prerequisites_and_requires_authoritative_readback() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        for readback_succeeds in [true, false] {
            let server = MockServer::start().await;
            let api = Api::mock(server.uri());
            let reserved = json!({"productId":"reserved","itemId":"reservation1","quantity":1,"c_ao_disponibilidad":"05"});
            mock_existing_cart(&server, json!({"basketId":"basket1","productItems":[]})).await;
            let deleted = Arc::new(AtomicBool::new(false));
            let changed = deleted.clone();
            Mock::given(method("DELETE"))
                .and(path("/checkout/shopper-baskets/v1/organizations/f_ecom_blzv_prd/baskets/basket1/items/line1"))
                .respond_with(move |_: &wiremock::Request| {
                    changed.store(true, Ordering::SeqCst);
                    ResponseTemplate::new(204)
                })
                .expect(1).mount(&server).await;
            let remaining = reserved.clone();
            Mock::given(method("GET"))
                .and(path(
                    "/checkout/shopper-baskets/v1/organizations/f_ecom_blzv_prd/baskets/basket1",
                ))
                .respond_with(move |_: &wiremock::Request| {
                    if deleted.load(Ordering::SeqCst) {
                        if readback_succeeds {
                            ResponseTemplate::new(200).set_body_json(
                                json!({"basketId":"basket1","productItems":[remaining]}),
                            )
                        } else {
                            ResponseTemplate::new(503)
                        }
                    } else {
                        ResponseTemplate::new(200).set_body_json(
                            json!({"basketId":"basket1","productItems":[
                                {"productId":"42171","itemId":"line1","quantity":1}, remaining
                            ]}),
                        )
                    }
                })
                .with_priority(1)
                .expect(2)
                .mount(&server)
                .await;
            // A discontinued product may no longer have a catalog record.
            Mock::given(method("GET"))
                .and(path(
                    "/product/shopper-products/v1/organizations/f_ecom_blzv_prd/products/42171",
                ))
                .respond_with(ResponseTemplate::new(404))
                .with_priority(1)
                .expect(0)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(
                    "/custom/saved-cart/v1/organizations/f_ecom_blzv_prd/sushi-reserve",
                ))
                .respond_with(ResponseTemplate::new(503))
                .with_priority(1)
                .expect(0)
                .mount(&server)
                .await;
            // Optional photos are fetched only after successful readback.
            Mock::given(method("GET"))
                .and(path(
                    "/product/shopper-products/v1/organizations/f_ecom_blzv_prd/products",
                ))
                .and(query_param("ids", "reserved"))
                .respond_with(ResponseTemplate::new(404))
                .expect(u64::from(readback_succeeds))
                .mount(&server)
                .await;
            let temp = tempfile::tempdir().unwrap();
            let store = Store {
                directory: temp.path().join("state"),
                backend: crate::store::CredentialStore::File,
            };
            let _lock = store.lock().await.unwrap();
            let result = set(
                &api,
                &session(&api),
                &store,
                "42171",
                Decimal::ZERO,
                Some("line1"),
            )
            .await;
            if readback_succeeds {
                let output = result.unwrap();
                assert_eq!(output["items"].as_array().unwrap().len(), 1);
                assert_eq!(output["items"][0]["product_id"], reserved["productId"]);
                assert_eq!(output["items"][0]["item_id"], reserved["itemId"]);
                assert_eq!(output["items"][0]["quantity"], reserved["quantity"]);
                assert!(store.pending().unwrap().is_none());
            } else {
                assert!(matches!(result, Err(Error::Uncertain)));
                assert_eq!(store.pending().unwrap().unwrap()["quantity"], "0");
            }
            let requests = server.received_requests().await.unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.method == "DELETE")
                    .count(),
                1
            );
            assert_eq!(requests.len(), if readback_succeeds { 6 } else { 5 });
            assert!(
                requests[..5]
                    .iter()
                    .all(|request| !request.url.path().contains("shopper-products"))
            );
        }
    }
    #[tokio::test]
    async fn no_op_uses_locale_and_optional_product_photos_without_writes() {
        let server = MockServer::start().await;
        let mut api = Api::mock(server.uri());
        api.locale = "ca".into();
        mock_existing_cart(
            &server,
            json!({"basketId":"basket1","productItems":[
                {"productId":"42171","itemId":"line1","quantity":1}
            ]}),
        )
        .await;
        Mock::given(method("GET"))
            .and(path(
                "/product/shopper-products/v1/organizations/f_ecom_blzv_prd/products",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{
                "id":"42171","image":{"link":"https://ametllerorigen.com/product.jpg"}
            }]})))
            .expect(1)
            .mount(&server)
            .await;
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: crate::store::CredentialStore::File,
        };
        let _lock = store.lock().await.unwrap();
        let output = set(&api, &session(&api), &store, "42171", Decimal::ONE, None)
            .await
            .unwrap();
        assert_eq!(output["checkout_url"], "https://ametllerorigen.com/ca/cart");
        assert_eq!(
            output["items"][0]["photo_urls"],
            json!(["https://ametllerorigen.com/product.jpg"])
        );
        assert!(store.pending().unwrap().is_none());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|r| r.method == "GET"));
    }
    #[tokio::test]
    async fn ambiguous_add_is_sent_once_and_guard_survives_a_new_store() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        mock_existing_cart(&server, json!({"basketId":"basket1","productItems":[]})).await;
        Mock::given(method("POST"))
            .and(path(
                "/checkout/shopper-baskets/v1/organizations/f_ecom_blzv_prd/baskets/basket1/items",
            ))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: crate::store::CredentialStore::File,
        };
        let _lock = store.lock().await.unwrap();
        assert!(matches!(
            set(&api, &session(&api), &store, "42171", Decimal::ONE, None).await,
            Err(Error::Uncertain)
        ));
        let reopened = store.clone();
        assert!(matches!(
            set(&api, &session(&api), &reopened, "42171", Decimal::ONE, None).await,
            Err(Error::Pending)
        ));
        assert!(
            get(&api, &session(&api), &reopened, false).await.unwrap()["pending_write"].is_object()
        );
    }
    #[tokio::test]
    async fn absolute_quantity_patch_is_read_back_and_clears_guard() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let server = MockServer::start().await;
        let mut api = Api::mock(server.uri());
        api.locale = "ca".into();
        mock_existing_cart(&server,json!({"basketId":"basket1","productItems":[{"productId":"42171","itemId":"line1","quantity":1}]})).await;
        let updated = Arc::new(AtomicBool::new(false));
        let changed = updated.clone();
        Mock::given(method("PATCH")).and(path("/checkout/shopper-baskets/v1/organizations/f_ecom_blzv_prd/baskets/basket1/items/line1"))
            .and(wiremock::matchers::body_json(json!({"quantity":2})))
            .respond_with(move |_: &wiremock::Request| { changed.store(true,Ordering::SeqCst);ResponseTemplate::new(200).set_body_json(json!({})) }).expect(1).mount(&server).await;
        Mock::given(method("GET")).and(path("/checkout/shopper-baskets/v1/organizations/f_ecom_blzv_prd/baskets/basket1"))
            .respond_with(move |_: &wiremock::Request|ResponseTemplate::new(200).set_body_json(json!({"basketId":"basket1","productItems":[{"productId":"42171","itemId":"line1","quantity":if updated.load(Ordering::SeqCst){2}else{1}}]})))
            .with_priority(1).expect(2).mount(&server).await;
        Mock::given(method("GET"))
            .and(path(
                "/product/shopper-products/v1/organizations/f_ecom_blzv_prd/products",
            ))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: crate::store::CredentialStore::File,
        };
        let _lock = store.lock().await.unwrap();
        let result = set(
            &api,
            &session(&api),
            &store,
            "42171",
            Decimal::from(2),
            Some("line1"),
        )
        .await
        .unwrap();
        assert_eq!(result["items"][0]["quantity"], 2);
        assert_eq!(result["checkout_url"], "https://ametllerorigen.com/ca/cart");
        assert!(store.pending().unwrap().is_none());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.url.path().ends_with("/customers/customer1/baskets"))
                .count(),
            2
        );
    }
}
