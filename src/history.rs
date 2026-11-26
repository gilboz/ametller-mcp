//! Account-scoped, normalized online-order history. Cached reads never use the network.
use crate::{
    Error, Result,
    api::{Api, id},
    auth::{Session, now},
    store::{Store, check_private},
};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::Method;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs::OpenOptions, path::PathBuf, str::FromStr, time::Duration};
use tokio::time::{Instant, timeout};

const PAGE_SIZE: usize = 50;
const MAX_ORDERS: usize = 10_000;
const MAX_LINES: usize = 1000;
const SCHEMA: i64 = 1;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Progress {
    account: String,
    run: String,
    next_offset: usize,
    expected_total: Option<usize>,
    in_progress: bool,
    last_attempt_at: Option<u64>,
    last_completed_at: Option<u64>,
    last_attempt_failed: bool,
}

struct CachedOrder {
    order_id: String,
    date: Option<String>,
    epoch: Option<i64>,
    status: Option<String>,
    payment_status: Option<String>,
    shipping_status: Option<String>,
    currency: Option<String>,
    total: Option<String>,
    lines: Vec<Line>,
}
struct Line {
    product_id: Option<String>,
    name: Option<String>,
    quantity: Option<String>,
    line_total: Option<String>,
    photo_url: Option<String>,
}

fn db<T>(result: rusqlite::Result<T>) -> Result<T> {
    result.map_err(|_| Error::HistoryStorage)
}

fn string(value: &Value, max: usize) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().filter(|c| !c.is_control()).take(max).collect())
}
fn decimal(value: &Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(Error::Response),
    };
    Decimal::from_str(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .map(|n| Some(n.normalize().to_string()))
        .map_err(|_| Error::Response)
}
fn first<'a>(value: &'a Value, fields: &[&str]) -> &'a Value {
    fields
        .iter()
        .find_map(|field| value.get(*field).filter(|v| !v.is_null()))
        .unwrap_or(&Value::Null)
}
fn normalized(value: &Value, expected_order: &str, customer: &str) -> Result<CachedOrder> {
    if value["orderNo"].as_str() != Some(expected_order)
        || value
            .pointer("/customerInfo/customerId")
            .and_then(Value::as_str)
            .is_some_and(|id| id != customer)
    {
        return Err(Error::Response);
    }
    let raw = value["productItems"].as_array().ok_or(Error::Response)?;
    if raw.len() > MAX_LINES {
        return Err(Error::Response);
    }
    let date = value["creationDate"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let mut lines = Vec::with_capacity(raw.len());
    for item in raw {
        let quantity = decimal(&item["quantity"])?;
        if quantity.as_deref().is_some_and(|q| q.starts_with('-')) {
            return Err(Error::Response);
        }
        let photo_url = item["c_VS_imageURL"].as_str().and_then(|s| {
            let url = url::Url::parse(s).ok()?;
            (url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some("ametllerorigen.com" | "www.ametllerorigen.com")
                )
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && s.len() <= 2048)
                .then(|| s.to_owned())
        });
        lines.push(Line {
            product_id: item["productId"]
                .as_str()
                .filter(|s| id(s).is_ok())
                .map(str::to_owned),
            name: string(&item["productName"], 500),
            quantity,
            line_total: decimal(first(
                item,
                &["priceAfterOrderDiscount", "priceAfterItemDiscount", "price"],
            ))?,
            photo_url,
        });
    }
    Ok(CachedOrder {
        order_id: expected_order.into(),
        date: date.map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, true)),
        epoch: date.map(|d| d.timestamp()),
        status: string(&value["status"], 80),
        payment_status: string(&value["paymentStatus"], 80),
        shipping_status: string(&value["shippingStatus"], 80),
        currency: string(&value["currency"], 10),
        total: decimal(&value["orderTotal"])?,
        lines,
    })
}

struct Cache {
    connection: Connection,
    progress: Progress,
}
fn cache_path(store: &Store, customer: &str) -> PathBuf {
    let hash = Sha256::digest(customer.as_bytes());
    let suffix: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    store.directory.join(format!("history-{suffix}.sqlite3"))
}
fn private_file(path: &std::path::Path) -> Result<()> {
    check_private(path, false)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if std::fs::symlink_metadata(path)
            .map_err(|_| Error::Storage)?
            .nlink()
            != 1
        {
            return Err(Error::Storage);
        }
    }
    Ok(())
}
impl Cache {
    // Caller holds Store::lock; this also verifies the private parent directory.
    fn open(store: &Store, customer: &str, create: bool) -> Result<Option<Self>> {
        id(customer)?;
        check_private(&store.directory, true)?;
        let file = cache_path(store, customer);
        let exists = match std::fs::symlink_metadata(&file) {
            Ok(_) => {
                private_file(&file)?;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(Error::Storage),
        };
        if !exists && !create {
            return Ok(None);
        }
        for suffix in ["-journal", "-wal", "-shm"] {
            let sidecar = file.with_file_name(format!(
                "{}{suffix}",
                file.file_name().unwrap().to_string_lossy()
            ));
            match std::fs::symlink_metadata(&sidecar) {
                Ok(_) => private_file(&sidecar)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::Storage),
            }
        }
        if !exists {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&file).map_err(|_| Error::Storage)?;
            private_file(&file)?;
        }
        let access = if create {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        // macOS exposes /var through a system symlink. The leaf was checked
        // above; use its canonical path so NOFOLLOW still protects the DB file.
        let canonical = file.canonicalize().map_err(|_| Error::Storage)?;
        let connection = db(Connection::open_with_flags(
            &canonical,
            access | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        ))?;
        db(connection.busy_timeout(Duration::from_secs(1)))?;
        db(connection.execute_batch(
            "PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;",
        ))?;
        let version: i64 = db(connection.pragma_query_value(None, "user_version", |r| r.get(0)))?;
        if version == 0 && create {
            db(connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
                BEGIN IMMEDIATE;
                CREATE TABLE IF NOT EXISTS meta (id INTEGER PRIMARY KEY CHECK(id=1), progress TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS orders (
                    order_id TEXT PRIMARY KEY, date TEXT, epoch INTEGER, status TEXT, payment_status TEXT,
                    shipping_status TEXT, currency TEXT, total TEXT, last_seen_run TEXT NOT NULL, synced_at INTEGER NOT NULL);
                CREATE TABLE IF NOT EXISTS order_items (
                    order_id TEXT NOT NULL REFERENCES orders(order_id) ON DELETE CASCADE,
                    line INTEGER NOT NULL, product_id TEXT, name TEXT, search_name TEXT NOT NULL,
                    quantity TEXT, line_total TEXT, photo_url TEXT, PRIMARY KEY(order_id,line));
                CREATE INDEX IF NOT EXISTS product_history ON order_items(product_id,order_id);
                PRAGMA user_version=1; COMMIT;"))?;
        } else if version != SCHEMA {
            return Err(Error::HistoryStorage);
        }
        let stored: Option<String> = db(connection
            .query_row("SELECT progress FROM meta WHERE id=1", [], |r| r.get(0))
            .optional())?;
        let progress = if let Some(stored) = stored {
            let parsed: Progress =
                serde_json::from_str(&stored).map_err(|_| Error::HistoryStorage)?;
            if parsed.account != customer {
                return Err(Error::HistoryStorage);
            }
            parsed
        } else if create {
            Progress {
                account: customer.into(),
                ..Progress::default()
            }
        } else {
            return Err(Error::HistoryStorage);
        };
        Ok(Some(Self {
            connection,
            progress,
        }))
    }
    fn save_progress(&self) -> Result<()> {
        let data = serde_json::to_string(&self.progress).map_err(|_| Error::HistoryStorage)?;
        db(self.connection.execute("INSERT INTO meta(id,progress) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET progress=excluded.progress", [data]))?;
        Ok(())
    }
    fn seen(&self, order: &str) -> Result<bool> {
        db(self
            .connection
            .query_row(
                "SELECT last_seen_run=?2 FROM orders WHERE order_id=?1",
                params![order, self.progress.run],
                |r| r.get(0),
            )
            .optional())
        .map(|v| v.unwrap_or(false))
    }
    fn store_order(&mut self, order: &CachedOrder, next_offset: usize) -> Result<()> {
        let tx = db(self.connection.transaction())?;
        db(tx.execute("INSERT INTO orders VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
            ON CONFLICT(order_id) DO UPDATE SET date=excluded.date,epoch=excluded.epoch,status=excluded.status,
            payment_status=excluded.payment_status,shipping_status=excluded.shipping_status,currency=excluded.currency,
            total=excluded.total,last_seen_run=excluded.last_seen_run,synced_at=excluded.synced_at",
            params![order.order_id,order.date,order.epoch,order.status,order.payment_status,order.shipping_status,
                order.currency,order.total,self.progress.run,i64::try_from(now()).map_err(|_|Error::HistoryStorage)?]))?;
        db(tx.execute(
            "DELETE FROM order_items WHERE order_id=?1",
            [&order.order_id],
        ))?;
        for (index, line) in order.lines.iter().enumerate() {
            db(tx.execute(
                "INSERT INTO order_items VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    order.order_id,
                    index as i64,
                    line.product_id,
                    line.name,
                    line.name.as_deref().unwrap_or("").to_lowercase(),
                    line.quantity,
                    line.line_total,
                    line.photo_url
                ],
            ))?;
        }
        let mut progress = self.progress.clone();
        progress.next_offset = next_offset;
        let data = serde_json::to_string(&progress).map_err(|_| Error::HistoryStorage)?;
        db(tx.execute("INSERT INTO meta(id,progress) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET progress=excluded.progress", [data]))?;
        db(tx.commit())?;
        self.progress = progress;
        Ok(())
    }
    fn metadata(&self) -> Result<Value> {
        let orders: i64 = db(self
            .connection
            .query_row("SELECT count(*) FROM orders", [], |r| r.get(0)))?;
        let products: i64 = db(self.connection.query_row(
            "SELECT count(DISTINCT product_id) FROM order_items",
            [],
            |r| r.get(0),
        ))?;
        let age = self
            .progress
            .last_completed_at
            .map(|t| now().saturating_sub(t));
        Ok(
            json!({"source":"local_sqlite","coverage":"online_orders_only","complete":self.progress.last_completed_at.is_some() && !self.progress.in_progress,
            "last_complete_sync_at":self.progress.last_completed_at,"last_attempt_at":self.progress.last_attempt_at,
            "last_attempt_failed":self.progress.last_attempt_failed,"age_seconds":age,
            "refresh_recommended":self.progress.in_progress || age.is_none_or(|v|v>86400),
            "cached_orders":orders,"cached_products":products,"reported_total":self.progress.expected_total,
            "next_offset":self.progress.in_progress.then_some(self.progress.next_offset),"retains_previously_seen_orders":true}),
        )
    }
}

fn empty_metadata() -> Value {
    json!({"source":"local_sqlite","coverage":"online_orders_only","complete":false,"initial_sync_required":true,
        "cached_orders":0,"cached_products":0,"last_complete_sync_at":null,"refresh_recommended":true})
}
pub fn cached_account(store: &Store) -> Result<String> {
    let metadata = store.metadata()?.ok_or(Error::LoginRequired)?;
    let customer = metadata["customer_id"].as_str().ok_or(Error::Storage)?;
    Ok(id(customer)?.into())
}

async fn get(
    api: &Api,
    session: &Session,
    service: &str,
    resource: &str,
    query: &[(&str, String)],
    deadline: Instant,
) -> Result<Value> {
    timeout(deadline.saturating_duration_since(Instant::now()),
        api.request(Method::GET,service,resource,&session.access_token,query,None))
        .await.map_err(|_| Error::Input("History refresh timed out. Cached progress was retained; run sync_purchase_history again to resume."))?
}

pub async fn sync(
    api: &Api,
    session: &Session,
    store: &Store,
    max_pages: u32,
    restart: bool,
) -> Result<Value> {
    if !(1..=100).contains(&max_pages) {
        return Err(Error::Input("max_pages must be 1-100."));
    }
    let mut cache = Cache::open(store, &session.customer_id, true)?.ok_or(Error::HistoryStorage)?;
    if restart || !cache.progress.in_progress {
        cache.progress.run = uuid::Uuid::new_v4().to_string();
        cache.progress.next_offset = 0;
        cache.progress.expected_total = None;
    }
    cache.progress.in_progress = true;
    cache.progress.last_attempt_at = Some(now());
    cache.progress.last_attempt_failed = false;
    cache.save_progress()?;
    let result = refresh(api, session, &mut cache, max_pages).await;
    if result.is_err() {
        cache.progress.last_attempt_failed = true;
    }
    cache.save_progress()?;
    let stop = result?;
    Ok(json!({"cache":cache.metadata()?,"stop_reason":stop,"remote_orders_modified":false}))
}

async fn refresh(
    api: &Api,
    session: &Session,
    cache: &mut Cache,
    max_pages: u32,
) -> Result<&'static str> {
    let deadline = Instant::now() + Duration::from_secs(20);
    for _ in 0..max_pages {
        if Instant::now() >= deadline {
            return Ok("time_budget");
        }
        let offset = cache.progress.next_offset;
        if offset >= MAX_ORDERS {
            return Ok("order_limit");
        }
        let response = get(
            api,
            session,
            "customer/shopper-customers",
            &format!("customers/{}/orders", id(&session.customer_id)?),
            &[
                ("offset", offset.to_string()),
                ("limit", PAGE_SIZE.to_string()),
            ],
            deadline,
        )
        .await?;
        // SCAPI's CustomerOrderResult makes data optional. Omission is an
        // empty history only when the server explicitly reports total == 0.
        // Missing/malformed rows with an unknown or nonzero total still fail.
        let rows: &[Value] = match response.get("data") {
            Some(Value::Array(rows)) => rows,
            None if response["total"].as_u64() == Some(0) => &[],
            _ => return Err(Error::Response),
        };
        if rows.len() > PAGE_SIZE
            || response["offset"]
                .as_u64()
                .is_some_and(|v| v != offset as u64)
        {
            return Err(Error::Input(
                "Order pagination changed. Cached purchases were retained; restart the history sync.",
            ));
        }
        let total = response
            .get("total")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or(Error::Response)
            })
            .transpose()?;
        if let (Some(old), Some(new)) = (cache.progress.expected_total, total)
            && old != new
        {
            return Err(Error::Input(
                "Order history changed during refresh. Restart the history sync for a consistent result.",
            ));
        }
        cache.progress.expected_total = total.or(cache.progress.expected_total);
        if cache
            .progress
            .expected_total
            .is_some_and(|n| offset + rows.len() > n)
        {
            return Err(Error::Response);
        }
        let mut page_ids = HashSet::new();
        for row in rows {
            let order = id(row["orderNo"].as_str().ok_or(Error::Response)?)?;
            if !page_ids.insert(order) || cache.seen(order)? {
                return Err(Error::Input(
                    "Order pagination repeated an order. Cached purchases were retained; restart the history sync.",
                ));
            }
        }
        for (index, row) in rows.iter().enumerate() {
            if Instant::now() >= deadline {
                return Ok("time_budget");
            }
            let order = row["orderNo"].as_str().unwrap();
            let detail;
            let value = if row["productItems"].is_array() {
                row
            } else {
                detail = get(
                    api,
                    session,
                    "checkout/shopper-orders",
                    &format!("orders/{order}"),
                    &[],
                    deadline,
                )
                .await?;
                &detail
            };
            let normalized = normalized(value, order, &session.customer_id)?;
            cache.store_order(&normalized, offset + index + 1)?;
        }
        let consumed = offset + rows.len();
        let complete = if let Some(total) = cache.progress.expected_total {
            consumed == total
        } else {
            rows.len() < PAGE_SIZE
        };
        if complete {
            cache.progress.in_progress = false;
            cache.progress.last_completed_at = Some(now());
            return Ok("complete");
        }
        if rows.len() < PAGE_SIZE {
            return Err(Error::Input(
                "The shop returned an incomplete order page. Cached progress was retained; refresh again.",
            ));
        }
    }
    Ok("page_budget")
}

fn paging(limit: u32, offset: u32) -> Result<()> {
    if (1..=50).contains(&limit) && offset <= MAX_ORDERS as u32 {
        Ok(())
    } else {
        Err(Error::Input(
            "History reads require limit 1-50 and offset 0-10000.",
        ))
    }
}
fn order_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"order_id":row.get::<_,String>(0)?,"ordered_at":row.get::<_,Option<String>>(1)?,
        "status":row.get::<_,Option<String>>(2)?,"payment_status":row.get::<_,Option<String>>(3)?,
        "shipping_status":row.get::<_,Option<String>>(4)?,"currency":row.get::<_,Option<String>>(5)?,
        "historical_total":row.get::<_,Option<String>>(6)?,"line_count":row.get::<_,i64>(7)?}),
    )
}
pub fn orders(
    store: &Store,
    customer: &str,
    order_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Value> {
    paging(limit, offset)?;
    if let Some(order) = order_id {
        id(order)?;
    }
    let Some(cache) = Cache::open(store, customer, false)? else {
        return Ok(
            json!({"cache":empty_metadata(),"orders":[],"instructions":"Run sync_purchase_history to populate this account's local history."}),
        );
    };
    let mut statement=db(cache.connection.prepare("SELECT o.order_id,o.date,o.status,o.payment_status,o.shipping_status,o.currency,o.total,
        (SELECT count(*) FROM order_items i WHERE i.order_id=o.order_id) FROM orders o
        WHERE (?1 IS NULL OR o.order_id=?1) ORDER BY o.epoch DESC,o.order_id DESC LIMIT ?2 OFFSET ?3"))?;
    let rows = db(statement.query_map(
        params![order_id, limit, if order_id.is_some() { 0 } else { offset }],
        order_json,
    ))?;
    let mut orders = db(rows.collect::<rusqlite::Result<Vec<_>>>())?;
    if let Some(order) = order_id {
        let found=orders.first_mut().ok_or(Error::Input("That order is not in this account's local history. Refresh history before looking it up."))?;
        let mut statement=db(cache.connection.prepare("SELECT product_id,name,quantity,line_total,photo_url FROM order_items WHERE order_id=?1 ORDER BY line"))?;
        let items=db(statement.query_map([order],|r|Ok(json!({"product_id":r.get::<_,Option<String>>(0)?,"name":r.get::<_,Option<String>>(1)?,
            "quantity":r.get::<_,Option<String>>(2)?,"historical_line_total":r.get::<_,Option<String>>(3)?,"photo_url":r.get::<_,Option<String>>(4)?}))))?;
        found["items"] = json!(db(items.collect::<rusqlite::Result<Vec<_>>>())?);
    }
    Ok(
        json!({"cache":cache.metadata()?,"orders":orders,"limit":limit,"offset":offset,"prices":"historical_not_current"}),
    )
}

pub fn products(
    store: &Store,
    customer: &str,
    query: &str,
    sort: &str,
    limit: u32,
    offset: u32,
) -> Result<Value> {
    paging(limit, offset)?;
    if query.chars().count() > 200 || !matches!(sort, "frequent" | "recent") {
        return Err(Error::Input(
            "Use a query of at most 200 characters and sort frequent or recent.",
        ));
    }
    let Some(cache) = Cache::open(store, customer, false)? else {
        return Ok(
            json!({"cache":empty_metadata(),"products":[],"instructions":"Run sync_purchase_history before making suggestions from past purchases."}),
        );
    };
    let order_by = if sort == "frequent" {
        "n.order_count DESC,c.epoch DESC,c.product_id"
    } else {
        "c.epoch DESC,n.order_count DESC,c.product_id"
    };
    let sql=format!("WITH candidates AS (
        SELECT i.*,o.date,o.epoch,o.currency,ROW_NUMBER() OVER(PARTITION BY i.product_id ORDER BY o.epoch DESC,o.order_id DESC,i.line) AS rank
        FROM order_items i JOIN orders o ON o.order_id=i.order_id
        WHERE i.product_id IS NOT NULL AND o.status IN ('new','open','completed')),
        counts AS (SELECT product_id,count(DISTINCT order_id) AS order_count FROM candidates GROUP BY product_id
            HAVING ?1='' OR max(instr(search_name,?1))>0)
        SELECT c.product_id,c.name,n.order_count,c.date,c.order_id,c.quantity,c.line_total,c.currency,c.photo_url,c.epoch
        FROM candidates c JOIN counts n ON c.product_id=n.product_id WHERE c.rank=1
        ORDER BY {order_by} LIMIT ?2 OFFSET ?3");
    let mut statement = db(cache.connection.prepare(&sql))?;
    let rows=db(statement.query_map(params![query.trim().to_lowercase(),limit,offset],|r|Ok(json!({
        "product_id":r.get::<_,String>(0)?,"name":r.get::<_,Option<String>>(1)?,"order_count":r.get::<_,i64>(2)?,
        "last_ordered_at":r.get::<_,Option<String>>(3)?,"last_order_id":r.get::<_,String>(4)?,
        "last_line_quantity":r.get::<_,Option<String>>(5)?,"historical_line_total":r.get::<_,Option<String>>(6)?,
        "currency":r.get::<_,Option<String>>(7)?,"photo_url":r.get::<_,Option<String>>(8)?,
        "current_availability":null,"current_price":null}))))?;
    let products = db(rows.collect::<rusqlite::Result<Vec<_>>>())?;
    Ok(
        json!({"cache":cache.metadata()?,"products":products,"query":query,"sort":sort,"limit":limit,"offset":offset,
        "included_order_statuses":["new","open","completed"],"prices":"historical_not_current",
        "instructions":"Suggestions only. Check current product details and quantity rules before a user-authorized cart change."}),
    )
}

pub fn clear(store: &Store, customer: &str) -> Result<()> {
    if Cache::open(store, customer, false)?.is_none() {
        return Ok(());
    }
    // Open/close validated the DB and its sidecars; account lock excludes writers.
    for suffix in ["-journal", "-wal", "-shm", ""] {
        let file = PathBuf::from(format!("{}{suffix}", cache_path(store, customer).display()));
        match std::fs::symlink_metadata(&file) {
            Ok(_) => {
                private_file(&file)?;
                std::fs::remove_file(file).map_err(|_| Error::Storage)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Storage),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{service::App, store::CredentialStore};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    fn store(root: &std::path::Path) -> Store {
        Store {
            directory: root.join("state"),
            backend: CredentialStore::File,
        }
    }
    fn session(api: &Api, customer: &str) -> Session {
        Session {
            access_token: "synthetic".into(),
            refresh_token: "synthetic-refresh".into(),
            customer_id: customer.into(),
            expires_at: now() + 3600,
            config: api.config.clone(),
        }
    }
    fn order(index: usize) -> Value {
        json!({"orderNo":format!("order-{index}"),"creationDate":"2026-10-01T10:00:00Z","status":"completed",
            "paymentStatus":"paid","shippingStatus":"shipped","currency":"EUR","orderTotal":3.17,
            "customerInfo":{"customerId":"customer1","email":"PRIVATE-EMAIL"},
            "billingAddress":{"address1":"PRIVATE-ADDRESS"},"paymentInstruments":[{"paymentCard":{"number":"PRIVATE-CARD"}}],
            "productItems":[{"productId":"2350","productName":"Yogur griego","quantity":1.5,"price":4.50,"priceAfterOrderDiscount":3.17,
                "c_VS_imageURL":"https://www.ametllerorigen.com/on/demandware.static/-/Sites-mastercatalog_AMETLLER/default/dw123/images/products/2350/2350.jpg"}]})
    }
    async fn page(server: &MockServer, offset: usize, total: usize, rows: Vec<Value>) {
        Mock::given(method("GET"))
            .and(path("/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/customer1/orders"))
            .and(query_param("siteId","ametller"))
            .and(query_param_is_missing("locale"))
            .and(query_param("offset",offset.to_string()))
            .and(query_param("limit","50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"offset":offset,"limit":50,"total":total,"data":rows})))
            .expect(1).mount(server).await;
    }
    #[tokio::test]
    async fn paginated_sync_resumes_and_replaces_orders_without_duplicates() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let session = session(&api, "customer1");
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        page(&server, 0, 52, (0..50).map(order).collect()).await;
        page(&server, 50, 52, (50..52).map(order).collect()).await;
        let first = sync(&api, &session, &store, 1, false).await.unwrap();
        assert_eq!(first["cache"]["complete"], false);
        assert_eq!(first["cache"]["next_offset"], 50);
        let last = sync(&api, &session, &store, 1, false).await.unwrap();
        assert_eq!(last["cache"]["complete"], true);
        assert_eq!(last["cache"]["cached_orders"], 52);
        let aggregated = products(&store, "customer1", "yogur", "frequent", 20, 0).unwrap();
        assert_eq!(aggregated["products"][0]["order_count"], 52);
        assert_eq!(aggregated["products"][0]["historical_line_total"], "3.17");
        assert_eq!(aggregated["products"][0]["last_line_quantity"], "1.5");
        server.verify().await;
        server.reset().await;
        let mut changed = order(0);
        changed["status"] = json!("cancelled");
        changed["productItems"] = json!([]);
        page(&server, 0, 1, vec![changed]).await;
        sync(&api, &session, &store, 1, true).await.unwrap();
        // A shrinking server retention window must not erase prior local purchases.
        assert_eq!(
            orders(&store, "customer1", None, 20, 0).unwrap()["cache"]["cached_orders"],
            52
        );
        assert!(
            orders(&store, "customer1", Some("order-0"), 20, 0).unwrap()["orders"][0]["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            products(&store, "customer1", "", "frequent", 20, 0).unwrap()["products"][0]["order_count"],
            51
        );
    }
    #[tokio::test]
    async fn failed_page_keeps_checkpoint_and_cached_reads_need_no_credentials() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let session = session(&api, "customer1");
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        {
            let _guard = store.lock().await.unwrap();
            store.save(&session).unwrap();
            page(&server, 0, 51, (0..50).map(order).collect()).await;
            Mock::given(method("GET"))
                .and(query_param("offset", "50"))
                .respond_with(ResponseTemplate::new(503))
                .expect(1)
                .mount(&server)
                .await;
            assert!(matches!(
                sync(&api, &session, &store, 2, false).await,
                Err(Error::Http(503))
            ));
            let cached = orders(&store, "customer1", None, 20, 0).unwrap();
            assert_eq!(cached["cache"]["cached_orders"], 50);
            assert_eq!(cached["cache"]["complete"], false);
            assert_eq!(cached["cache"]["last_attempt_failed"], true);
            std::fs::remove_file(store.directory.join("credentials.json")).unwrap();
        }
        // If these paths load credentials or use the network they cannot succeed.
        let app = App::new(store.clone(), "es".into()).unwrap();
        let result = app.purchase_history(Some("order-1"), 20, 0).await.unwrap();
        assert_eq!(result["orders"][0]["items"][0]["quantity"], "1.5");
        assert!(
            app.purchased_products("yogur", "recent", 20, 0)
                .await
                .is_ok()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        server.verify().await;
        server.reset().await;
        page(&server, 50, 51, vec![order(50)]).await;
        let _guard = store.lock().await.unwrap();
        assert_eq!(
            sync(&api, &session, &store, 2, false).await.unwrap()["cache"]["complete"],
            true
        );
    }
    #[tokio::test]
    async fn repeated_pages_never_claim_complete_history() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let session = session(&api, "customer1");
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        page(&server, 0, 100, (0..50).map(order).collect()).await;
        page(&server, 50, 100, (0..50).map(order).collect()).await;
        assert!(
            matches!(sync(&api,&session,&store,2,false).await,Err(Error::Input(message)) if message.contains("repeated"))
        );
        let metadata = orders(&store, "customer1", None, 20, 0).unwrap()["cache"].clone();
        assert_eq!(metadata["complete"], false);
        assert_eq!(metadata["cached_orders"], 50);
    }
    #[tokio::test]
    async fn missing_lines_use_read_only_detail_and_reject_account_mismatch() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let session = session(&api, "customer1");
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        let mut summary = order(1);
        summary.as_object_mut().unwrap().remove("productItems");
        page(&server, 0, 1, vec![summary.clone()]).await;
        Mock::given(method("GET"))
            .and(path(
                "/checkout/shopper-orders/v1/organizations/f_ecom_blzv_prd/orders/order-1",
            ))
            .and(query_param("siteId", "ametller"))
            .respond_with(ResponseTemplate::new(200).set_body_json(order(1)))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            sync(&api, &session, &store, 1, false).await.unwrap()["cache"]["cached_products"],
            1
        );
        server.verify().await;
        server.reset().await;
        page(&server, 0, 1, vec![summary]).await;
        let mut foreign = order(1);
        foreign["customerInfo"]["customerId"] = json!("someone-else");
        Mock::given(method("GET"))
            .and(path(
                "/checkout/shopper-orders/v1/organizations/f_ecom_blzv_prd/orders/order-1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(foreign))
            .expect(1)
            .mount(&server)
            .await;
        assert!(sync(&api, &session, &store, 1, true).await.is_err());
        assert_eq!(
            orders(&store, "customer1", Some("order-1"), 20, 0).unwrap()["orders"][0]["items"][0]["name"],
            "Yogur griego"
        );
    }
    #[tokio::test]
    async fn normalization_keeps_exact_unknowns_and_excludes_private_checkout_fields() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        let mut cache = Cache::open(&store, "customer1", true).unwrap().unwrap();
        let mut raw = order(1);
        raw["orderTotal"] = Value::Null;
        raw["productSubTotal"] = json!(3.17);
        raw["productItems"][0]["priceAfterOrderDiscount"] = json!(0);
        raw["productItems"][0]["quantity"] = Value::Null;
        cache
            .store_order(&normalized(&raw, "order-1", "customer1").unwrap(), 1)
            .unwrap();
        drop(cache);
        let result = orders(&store, "customer1", Some("order-1"), 20, 0).unwrap();
        let item = &result["orders"][0]["items"][0];
        assert_eq!(item["historical_line_total"], "0");
        assert!(item["quantity"].is_null());
        assert!(result["orders"][0]["historical_total"].is_null());
        assert!(
            !String::from_utf8_lossy(&std::fs::read(cache_path(&store, "customer1")).unwrap())
                .contains("PRIVATE-")
        );
        assert!(!result.to_string().contains("customerInfo"));
    }
    #[tokio::test]
    async fn account_files_are_isolated_private_and_do_not_accept_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        let mut cache = Cache::open(&store, "customer1", true).unwrap().unwrap();
        cache
            .store_order(&normalized(&order(1), "order-1", "customer1").unwrap(), 1)
            .unwrap();
        drop(cache);
        assert_eq!(
            orders(&store, "customer2", None, 20, 0).unwrap()["cache"]["initial_sync_required"],
            true
        );
        std::fs::copy(
            cache_path(&store, "customer1"),
            cache_path(&store, "customer2"),
        )
        .unwrap();
        assert!(matches!(
            orders(&store, "customer2", None, 20, 0),
            Err(Error::HistoryStorage)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, symlink};
            assert_eq!(
                std::fs::metadata(cache_path(&store, "customer1"))
                    .unwrap()
                    .mode()
                    & 0o777,
                0o600
            );
            symlink(
                cache_path(&store, "customer1"),
                cache_path(&store, "customer3"),
            )
            .unwrap();
            assert!(matches!(
                orders(&store, "customer3", None, 20, 0),
                Err(Error::Storage)
            ));
        }
        clear(&store, "customer1").unwrap();
        assert!(!cache_path(&store, "customer1").exists());
        assert!(cache_path(&store, "customer2").exists());
    }
    #[tokio::test]
    async fn failed_line_write_rolls_back_header_and_previous_lines() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        let mut cache = Cache::open(&store, "customer1", true).unwrap().unwrap();
        cache
            .store_order(&normalized(&order(1), "order-1", "customer1").unwrap(), 1)
            .unwrap();
        cache.connection.execute_batch("CREATE TRIGGER test_failure BEFORE INSERT ON order_items WHEN NEW.line=1 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        let mut replacement = order(1);
        replacement["status"] = json!("cancelled");
        let second = replacement["productItems"][0].clone();
        replacement["productItems"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(
            cache
                .store_order(
                    &normalized(&replacement, "order-1", "customer1").unwrap(),
                    2
                )
                .is_err()
        );
        drop(cache);
        let result = orders(&store, "customer1", Some("order-1"), 20, 0).unwrap();
        assert_eq!(result["orders"][0]["status"], "completed");
        assert_eq!(result["orders"][0]["items"].as_array().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn failed_commit_does_not_advance_the_resumable_checkpoint() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        let mut cache = Cache::open(&store, "customer1", true).unwrap().unwrap();
        cache
            .store_order(&normalized(&order(1), "order-1", "customer1").unwrap(), 1)
            .unwrap();
        // This constraint is checked only at COMMIT, after the metadata write.
        cache.connection.execute_batch("CREATE TABLE test_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE test_child(id INTEGER REFERENCES test_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER test_commit_failure AFTER INSERT ON order_items WHEN NEW.order_id='order-2'
            BEGIN INSERT INTO test_child VALUES(999); END;").unwrap();
        assert!(
            cache
                .store_order(&normalized(&order(2), "order-2", "customer1").unwrap(), 2)
                .is_err()
        );
        assert_eq!(cache.progress.next_offset, 1);
        // The outer sync records the failure after a transaction error.
        cache.progress.last_attempt_failed = true;
        cache.save_progress().unwrap();
        drop(cache);
        let cache = Cache::open(&store, "customer1", false).unwrap().unwrap();
        assert_eq!(cache.progress.next_offset, 1);
        assert_eq!(cache.metadata().unwrap()["cached_orders"], 1);
        assert!(!cache.seen("order-2").unwrap());
    }
    #[tokio::test]
    async fn searching_an_old_name_keeps_all_orders_and_the_latest_product_details() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        let mut cache = Cache::open(&store, "customer1", true).unwrap().unwrap();
        cache
            .store_order(&normalized(&order(1), "order-1", "customer1").unwrap(), 1)
            .unwrap();
        let mut renamed = order(2);
        renamed["creationDate"] = json!("2026-11-01T11:00:00+01:00");
        renamed["productItems"][0]["productName"] = json!("Iogurt grec");
        renamed["productItems"][0]["quantity"] = json!(3);
        cache
            .store_order(&normalized(&renamed, "order-2", "customer1").unwrap(), 2)
            .unwrap();
        drop(cache);
        let found = products(&store, "customer1", "Yogur", "frequent", 20, 0).unwrap();
        let product = &found["products"][0];
        assert_eq!(product["order_count"], 2);
        assert_eq!(product["name"], "Iogurt grec");
        assert_eq!(product["last_order_id"], "order-2");
        assert_eq!(product["last_ordered_at"], "2026-11-01T10:00:00Z");
        assert_eq!(product["last_line_quantity"], "3");
    }
    #[tokio::test]
    async fn empty_sync_differs_from_missing_cache_and_unknown_statuses_do_not_suggest() {
        let server = MockServer::start().await;
        let api = Api::mock(server.uri());
        let session = session(&api, "customer1");
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let _guard = store.lock().await.unwrap();
        assert_eq!(
            orders(&store, "customer1", None, 20, 0).unwrap()["cache"]["initial_sync_required"],
            true
        );
        page(&server, 0, 0, vec![]).await;
        assert_eq!(
            sync(&api, &session, &store, 1, false).await.unwrap()["cache"]["complete"],
            true
        );
        let mut cache = Cache::open(&store, "customer1", true).unwrap().unwrap();
        let mut raw = order(1);
        raw["status"] = json!("unknown_status");
        cache
            .store_order(&normalized(&raw, "order-1", "customer1").unwrap(), 1)
            .unwrap();
        drop(cache);
        assert!(
            products(&store, "customer1", "", "frequent", 20, 0).unwrap()["products"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            orders(&store, "customer1", None, 20, 0).unwrap()["orders"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[tokio::test]
    async fn omitted_order_array_is_empty_only_with_an_explicit_zero_total() {
        for (response, is_empty) in [
            (json!({"limit":50,"offset":0,"total":0}), true),
            (json!({"limit":50,"offset":0,"total":1}), false),
            (json!({"limit":50,"offset":0}), false),
            (json!({"limit":50,"offset":0,"total":0,"data":{}}), false),
        ] {
            let server = MockServer::start().await;
            let api = Api::mock(server.uri());
            let session = session(&api, "customer1");
            let root = tempfile::tempdir().unwrap();
            let store = store(root.path());
            let _guard = store.lock().await.unwrap();
            Mock::given(method("GET"))
                .and(path("/customer/shopper-customers/v1/organizations/f_ecom_blzv_prd/customers/customer1/orders"))
                .respond_with(ResponseTemplate::new(200).set_body_json(response))
                .expect(1).mount(&server).await;
            let result = sync(&api, &session, &store, 1, false).await;
            if is_empty {
                let result = result.unwrap();
                assert_eq!(result["cache"]["complete"], true);
                assert_eq!(result["cache"]["reported_total"], 0);
                assert_eq!(result["stop_reason"], "complete");
            } else {
                assert!(matches!(result, Err(Error::Response)));
                assert_eq!(
                    orders(&store, "customer1", None, 20, 0).unwrap()["cache"]["complete"],
                    false
                );
            }
        }
    }
}
