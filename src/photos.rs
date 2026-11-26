//! Optional public product thumbnails for MCP responses. Never uses account credentials.
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder};
use reqwest::Client;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};
use std::{io::Cursor, time::Duration};
use tokio::{sync::Semaphore, task::JoinSet};
use url::Url;

pub const MAX_SEARCH_PHOTOS: usize = 8;
pub const MAX_DETAIL_PHOTOS: usize = 3;
pub const MAX_PREVIEW_BYTES: usize = 192 * 1024;
pub const MAX_TOTAL_BASE64_BYTES: usize = 1536 * 1024;
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PIXELS: u64 = 8 * 1024 * 1024;
const PREVIEW_EDGE: u32 = 512;
// Includes downloads and decoding, across simultaneous tool calls. A cancelled
// request keeps its decoding permit until the blocking decoder has finished.
static PHOTO_SLOTS: Semaphore = Semaphore::const_new(3);

#[derive(Clone)]
struct PhotoClient {
    http: Client,
    #[cfg(test)]
    test_origin: Option<Url>,
}
struct Preview {
    data: String,
    width: u32,
    height: u32,
}
struct Candidate {
    product_index: usize,
    url: Url,
    caption: String,
}

fn photo_url(value: &str) -> Option<Url> {
    if value.len() > 2048 || value.contains(['%', '\\']) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    let path = url.path();
    (url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some("ametllerorigen.com" | "www.ametllerorigen.com")
        )
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && path.starts_with("/on/demandware.static/-/Sites-mastercatalog_AMETLLER/default/")
        && path.contains("/images/products/")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
        && [".jpg", ".jpeg", ".png", ".webp"]
            .iter()
            .any(|extension| path.ends_with(extension)))
    .then_some(url)
}

impl PhotoClient {
    fn new() -> Option<Self> {
        Some(Self {
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(6))
                .user_agent(concat!("ametller-mcp/", env!("CARGO_PKG_VERSION")))
                .build()
                .ok()?,
            #[cfg(test)]
            test_origin: None,
        })
    }
    async fn fetch(&self, url: Url) -> Option<Preview> {
        let permit = PHOTO_SLOTS.acquire().await.ok()?;
        #[cfg(test)]
        let url = self
            .test_origin
            .as_ref()
            .map_or_else(|| url.clone(), |origin| origin.join(url.path()).unwrap());
        let mut response = self.http.get(url).send().await.ok()?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > MAX_SOURCE_BYTES as u64)
        {
            return None;
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)?
            .to_str()
            .ok()?;
        let format = match mime.split(';').next()?.trim().to_ascii_lowercase().as_str() {
            "image/jpeg" => ImageFormat::Jpeg,
            "image/png" => ImageFormat::Png,
            "image/webp" => ImageFormat::WebP,
            _ => return None,
        };
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if chunk.len() > MAX_SOURCE_BYTES.saturating_sub(body.len()) {
                return None;
            }
            body.extend_from_slice(&chunk);
        }
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            thumbnail(&body, format)
        })
        .await
        .ok()?
    }
}

fn thumbnail(bytes: &[u8], format: ImageFormat) -> Option<Preview> {
    if bytes.len() > MAX_SOURCE_BYTES || image::guess_format(bytes).ok()? != format {
        return None;
    }
    // Apply the allocation limit before parsing headers too: PNG metadata can
    // decompress during the dimension probe, before pixel decoding begins.
    let (width, height) = limited_reader(bytes, format).into_dimensions().ok()?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return None;
    }
    let resized = limited_reader(bytes, format)
        .decode()
        .ok()?
        .thumbnail(PREVIEW_EDGE, PREVIEW_EDGE);
    // Composite transparent source pixels on white before JPEG encoding.
    let rgba = resized.to_rgba8();
    let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y).0;
        let alpha = u16::from(pixel[3]);
        image::Rgb(
            [0, 1, 2]
                .map(|i| ((u16::from(pixel[i]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8),
        )
    });
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 78)
        .encode_image(&rgb)
        .ok()?;
    if encoded.len() > MAX_PREVIEW_BYTES {
        return None;
    }
    Some(Preview {
        data: STANDARD.encode(encoded),
        width: rgb.width(),
        height: rgb.height(),
    })
}

fn limited_reader(bytes: &[u8], format: ImageFormat) -> ImageReader<Cursor<&[u8]>> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader
}

/// Structured catalog metadata remains available even if every photo fails.
/// Image data is in protocol image blocks, never duplicated in JSON text.
pub async fn tool_result(tool: &str, value: Value) -> CallToolResult {
    if !matches!(tool, "search_products" | "get_product") {
        return CallToolResult::structured(value);
    }
    catalog_result(tool, value, PhotoClient::new()).await
}

fn product_mut(value: &mut Value, search: bool, index: usize) -> Option<&mut Value> {
    if search {
        value.get_mut("products")?.as_array_mut()?.get_mut(index)
    } else {
        Some(value)
    }
}

async fn catalog_result(
    tool: &str,
    mut value: Value,
    client: Option<PhotoClient>,
) -> CallToolResult {
    let search = tool == "search_products";
    let count = if search {
        value["products"].as_array().map_or(0, Vec::len)
    } else {
        1
    };
    let limit = if search {
        MAX_SEARCH_PHOTOS
    } else {
        MAX_DETAIL_PHOTOS
    };
    let mut candidates = Vec::new();
    for index in 0..count {
        let Some(product) = product_mut(&mut value, search, index) else {
            continue;
        };
        let urls: Vec<_> = product["photo_urls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(photo_url)
            .collect();
        let status = if !urls.is_empty() && candidates.len() >= limit {
            "limit_reached"
        } else {
            "unavailable"
        };
        product["photo_previews"] = json!({"status":status,"images":[]});
        let take = if search { 1 } else { MAX_DETAIL_PHOTOS };
        for url in urls.into_iter().take(take) {
            if candidates.len() >= limit {
                break;
            }
            let name: String = product["name"]
                .as_str()
                .unwrap_or("Product")
                .chars()
                .take(300)
                .collect();
            let id: String = product["product_id"]
                .as_str()
                .unwrap_or("unknown")
                .chars()
                .take(150)
                .collect();
            candidates.push(Candidate {
                product_index: index,
                url,
                caption: format!("{name} (product {id})"),
            });
        }
    }
    let mut jobs = JoinSet::new();
    if let Some(client) = client {
        for (index, candidate) in candidates.iter().enumerate() {
            let client = client.clone();
            let url = candidate.url.clone();
            jobs.spawn(async move {
                (
                    index,
                    tokio::time::timeout(Duration::from_secs(12), client.fetch(url))
                        .await
                        .ok()
                        .flatten(),
                )
            });
        }
    }
    let mut images = std::iter::repeat_with(|| None)
        .take(candidates.len())
        .collect::<Vec<_>>();
    while let Some(result) = jobs.join_next().await {
        if let Ok((index, image)) = result {
            images[index] = image;
        }
    }
    assemble(value, search, candidates, images)
}

fn assemble(
    mut value: Value,
    search: bool,
    candidates: Vec<Candidate>,
    images: Vec<Option<Preview>>,
) -> CallToolResult {
    let mut content = Vec::new();
    let mut image_bytes = 0;
    for (candidate, image) in candidates.into_iter().zip(images) {
        let Some(image) = image else { continue };
        let product = product_mut(&mut value, search, candidate.product_index).unwrap();
        if image.data.len() > MAX_TOTAL_BASE64_BYTES - image_bytes {
            if product["photo_previews"]["status"] != "embedded" {
                product["photo_previews"]["status"] = json!("limit_reached");
            }
            continue;
        }
        image_bytes += image.data.len();
        // structured() adds the JSON text at index zero; each caption precedes its image.
        let content_index = content.len() + 2;
        product["photo_previews"]["status"] = json!("embedded");
        product["photo_previews"]["images"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "content_index":content_index,"source_url":candidate.url.as_str(),
                "mime_type":"image/jpeg","width":image.width,"height":image.height,
            }));
        content.push(ContentBlock::text(candidate.caption));
        content.push(ContentBlock::image(image.data, "image/jpeg"));
    }
    let mut result = CallToolResult::structured(value);
    result.content.extend(content);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageDecoder;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn official_path(product: usize) -> String {
        format!(
            "/on/demandware.static/-/Sites-mastercatalog_AMETLLER/default/dw123/images/products/{product}/{product}_1.jpg"
        )
    }
    fn official_url(product: usize) -> String {
        format!("https://www.ametllerorigen.com{}", official_path(product))
    }
    fn jpeg() -> Vec<u8> {
        let image = image::RgbImage::from_pixel(1000, 600, image::Rgb([10, 20, 30]));
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 80)
            .encode_image(&image)
            .unwrap();
        bytes
    }
    fn client(server: &MockServer) -> PhotoClient {
        PhotoClient {
            test_origin: Some(Url::parse(&server.uri()).unwrap()),
            ..PhotoClient::new().unwrap()
        }
    }
    #[test]
    fn only_public_retailer_product_files_are_fetchable() {
        assert!(photo_url(&official_url(2350)).is_some());
        for url in [
            "http://www.ametllerorigen.com/on/demandware.static/-/Sites-mastercatalog_AMETLLER/default/dw123/images/products/1/1.jpg",
            "https://www.ametllerorigen.com/mobify/proxy/api/customer/profile.jpg",
            "https://127.0.0.1/image.jpg",
            "https://www.ametllerorigen.com.evil.test/image.jpg",
        ] {
            assert!(photo_url(url).is_none(), "{url}");
        }
        for modified in [
            official_url(1).replace("https://", "https://user:secret@"),
            official_url(1).replace(".com/", ".com:8443/"),
            format!("{}?token=secret", official_url(1)),
            format!("{}#fragment", official_url(1)),
            official_url(1).replace("1_1.jpg", "%2e%2e/private.jpg"),
            official_url(1).replace("1_1.jpg", "1.svg"),
        ] {
            assert!(photo_url(&modified).is_none(), "{modified}");
        }
    }
    #[test]
    fn decoded_previews_are_bounded_valid_jpeg_and_mime_checked() {
        let bytes = jpeg();
        let preview = thumbnail(&bytes, ImageFormat::Jpeg).unwrap();
        let decoded = STANDARD.decode(&preview.data).unwrap();
        assert!(decoded.len() <= MAX_PREVIEW_BYTES);
        let image = image::load_from_memory(&decoded).unwrap();
        assert_eq!((image.width(), image.height()), (512, 307));
        assert_eq!((preview.width, preview.height), (512, 307));
        assert!(thumbnail(&bytes, ImageFormat::Png).is_none());
        assert!(thumbnail(b"not an image", ImageFormat::Jpeg).is_none());
        let oversized = image::RgbImage::new(4097, 1);
        let mut bytes = Vec::new();
        JpegEncoder::new(&mut bytes)
            .encode_image(&oversized)
            .unwrap();
        assert!(thumbnail(&bytes, ImageFormat::Jpeg).is_none());
    }
    #[test]
    fn compressed_metadata_is_limited_before_dimension_probe() {
        // A 1x1 PNG with a synthetic 65 MiB ICC profile compressed to <70 KiB.
        // The PNG parser expands this profile while obtaining dimensions.
        let bytes = include_bytes!("../tests/fixtures/oversized-image-metadata.png");
        assert!(bytes.len() < 70 * 1024);
        let mut decoder = limited_reader(bytes, ImageFormat::Png)
            .into_decoder()
            .unwrap();
        assert_eq!(decoder.dimensions(), (1, 1));
        // png discards an over-budget ancillary ICC profile while preserving
        // valid pixels. With default limits this profile would be retained.
        assert!(decoder.icc_profile().unwrap().is_none());
        assert!(thumbnail(bytes, ImageFormat::Png).is_some());
    }
    #[test]
    fn aggregate_image_budget_preserves_labels_and_marks_omitted_previews() {
        let mut jpeg = STANDARD
            .decode(thumbnail(&jpeg(), ImageFormat::Jpeg).unwrap().data)
            .unwrap();
        // JPEG permits bytes after EOI; use valid padded images at the individual
        // payload ceiling to exercise the aggregate wire-size boundary.
        jpeg.resize(MAX_PREVIEW_BYTES, 0);
        assert!(image::load_from_memory(&jpeg).is_ok());
        let data = STANDARD.encode(jpeg);
        let products: Vec<_> = (0..8).map(|id| json!({"product_id":id.to_string(),"photo_previews":{"status":"unavailable","images":[]}})).collect();
        let candidates = (0..8)
            .map(|id| Candidate {
                product_index: id,
                url: photo_url(&official_url(id)).unwrap(),
                caption: format!("Product {id}"),
            })
            .collect();
        let images = (0..8)
            .map(|_| {
                Some(Preview {
                    data: data.clone(),
                    width: 512,
                    height: 307,
                })
            })
            .collect();
        let result = assemble(json!({"products":products}), true, candidates, images);
        let wire = serde_json::to_value(result).unwrap();
        let embedded: Vec<_> = wire["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "image")
            .collect();
        assert_eq!(embedded.len(), 6);
        assert!(
            embedded
                .iter()
                .map(|item| item["data"].as_str().unwrap().len())
                .sum::<usize>()
                <= MAX_TOTAL_BASE64_BYTES
        );
        assert!(serde_json::to_vec(&wire).unwrap().len() < MAX_TOTAL_BASE64_BYTES + 16384);
        assert_eq!(
            wire["structuredContent"]["products"][6]["photo_previews"]["status"],
            "limit_reached"
        );
        assert_eq!(
            wire["structuredContent"]["products"][7]["photo_previews"]["status"],
            "limit_reached"
        );
    }
    #[tokio::test]
    async fn search_keeps_identity_json_and_embeds_at_most_eight_images_without_credentials() {
        let server = MockServer::start().await;
        let bytes = jpeg();
        Mock::given(method("GET"))
            .respond_with(move |request: &wiremock::Request| {
                assert!(!request.headers.contains_key("authorization"));
                assert!(!request.headers.contains_key("cookie"));
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "image/jpeg")
                    .set_body_bytes(bytes.clone())
            })
            .expect(MAX_SEARCH_PHOTOS as u64)
            .mount(&server)
            .await;
        let products: Vec<_> = (0..12).map(|id| json!({"product_id":id.to_string(),"name":format!("Yogurt {id}"),"price":1.39,"photo_urls":[official_url(id)]})).collect();
        let result = catalog_result(
            "search_products",
            json!({"products":products}),
            Some(client(&server)),
        )
        .await;
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["isError"], false);
        assert_eq!(
            wire["content"].as_array().unwrap().len(),
            1 + 2 * MAX_SEARCH_PHOTOS
        );
        assert!(serde_json::to_vec(&result).unwrap().len() < 3 * 1024 * 1024);
        let structured = &wire["structuredContent"];
        for id in 0..12 {
            let product = &structured["products"][id];
            assert_eq!(product["price"], 1.39);
            assert_eq!(product["photo_urls"][0], official_url(id));
            if id < MAX_SEARCH_PHOTOS {
                let index = product["photo_previews"]["images"][0]["content_index"]
                    .as_u64()
                    .unwrap() as usize;
                assert_eq!(wire["content"][index]["type"], "image");
                assert_eq!(wire["content"][index]["mimeType"], "image/jpeg");
                assert!(
                    wire["content"][index - 1]["text"]
                        .as_str()
                        .unwrap()
                        .contains(&format!("product {id}"))
                );
                assert!(
                    image::load_from_memory(
                        &STANDARD
                            .decode(wire["content"][index]["data"].as_str().unwrap())
                            .unwrap()
                    )
                    .is_ok()
                );
            } else {
                assert_eq!(product["photo_previews"]["status"], "limit_reached");
            }
        }
        assert_eq!(
            serde_json::from_str::<Value>(wire["content"][0]["text"].as_str().unwrap()).unwrap(),
            *structured
        );
    }
    #[tokio::test]
    async fn redirects_oversized_and_invalid_images_fall_back_without_failing_product() {
        let server = MockServer::start().await;
        for (id, response) in [
            (
                0,
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/private", server.uri())),
            ),
            (
                1,
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "image/jpeg")
                    .set_body_bytes(vec![0; MAX_SOURCE_BYTES + 1]),
            ),
            (
                2,
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "image/svg+xml")
                    .set_body_string("<svg/>"),
            ),
            (
                3,
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "image/jpeg")
                    .set_body_string("not a jpeg"),
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(official_path(id)))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            let result = catalog_result("get_product", json!({"product_id":id.to_string(),"name":"Yogurt","photo_urls":[official_url(id)]}), Some(client(&server))).await;
            assert_eq!(result.is_error, Some(false));
            assert_eq!(result.content.len(), 1);
            assert_eq!(
                result.structured_content.unwrap()["photo_previews"]["status"],
                "unavailable"
            );
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 4);
    }
    #[tokio::test]
    async fn detail_has_three_photos_and_other_tools_are_unchanged() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "image/jpeg")
                    .set_body_bytes(jpeg()),
            )
            .expect(MAX_DETAIL_PHOTOS as u64)
            .mount(&server)
            .await;
        let value = json!({"product_id":"1","name":"Yogurt","photo_urls":(0..6).map(official_url).collect::<Vec<_>>()});
        let result = catalog_result("get_product", value.clone(), Some(client(&server))).await;
        assert_eq!(result.content.len(), 1 + 2 * MAX_DETAIL_PHOTOS);
        assert_eq!(
            tool_result("get_cart", value.clone()).await,
            CallToolResult::structured(value)
        );
    }
}
