//! Local Ametller catalog, private CLI authentication, and cart MCP.

pub mod api;
pub mod auth;
pub mod cart;
pub mod error;
pub mod mcp;
pub mod service;
pub mod store;
pub use error::{Error, Result};

use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDetail {
    pub id: String,
    pub name: String,
    pub currency: Option<String>,
    pub price: Option<Number>,
    pub price_per_unit: Option<Number>,
    pub unit_measure: Option<String>,
    pub unit_quantity: Option<Number>,
    pub min_order_quantity: Option<Number>,
    pub step_quantity: Option<Number>,
    pub short_description: Option<String>,
    pub long_description: Option<String>,
    pub slug_url: Option<String>,
    pub inventory: Option<Inventory>,
    #[serde(default)]
    pub image_groups: Vec<ImageGroup>,
    #[serde(rename = "c_ao_pesoAproximado")]
    pub approximate_weight: Option<Number>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Inventory {
    /// Catalog availability only, not a delivery-address stock promise.
    pub orderable: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGroup {
    pub view_type: Option<String>,
    #[serde(default)]
    pub images: Vec<ProductImage>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductImage {
    pub link: Option<String>,
    pub dis_base_link: Option<String>,
    pub alt: Option<String>,
}
