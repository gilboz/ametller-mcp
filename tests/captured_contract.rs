use ametller_mcp::ProductDetail;

#[test]
fn captured_weighted_product_preserves_purchase_unit_and_estimated_weight() {
    let product: ProductDetail =
        serde_json::from_str(include_str!("fixtures/product-weighted.json")).unwrap();
    assert_eq!(product.id, "42171");
    assert_eq!(product.currency.as_deref(), Some("EUR"));
    assert_eq!(product.price.unwrap().to_string(), "3.41");
    assert_eq!(product.price_per_unit.unwrap().to_string(), "3.79");
    assert_eq!(product.unit_measure.as_deref(), Some("KG"));
    assert_eq!(product.approximate_weight.unwrap().to_string(), "0.9");
    // One bunch is the API purchase increment, not 0.9 items or 0.9 kg ordered.
    assert_eq!(product.min_order_quantity.unwrap().to_string(), "1");
    assert_eq!(product.step_quantity.unwrap().to_string(), "1");
    assert_eq!(product.inventory.unwrap().orderable, Some(true));
    assert!(product.short_description.is_some());
    assert!(product.image_groups.iter().any(|group| {
        group
            .images
            .iter()
            .any(|image| image.link.is_some() || image.dis_base_link.is_some())
    }));
}

#[test]
fn missing_inventory_is_unknown_not_available() {
    let product: ProductDetail =
        serde_json::from_str(r#"{"id":"example","name":"Example"}"#).unwrap();
    assert!(product.inventory.is_none());
    assert!(product.price.is_none());
    assert!(product.approximate_weight.is_none());
}
