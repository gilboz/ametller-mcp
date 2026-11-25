use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Input(&'static str),
    #[error("The shop returned HTTP {0}. No request was retried.")]
    Http(u16),
    #[error("Network request failed or timed out. No request was retried.")]
    Network,
    #[error("The shop returned an invalid or oversized response.")]
    Response,
    #[error("Could not access the private local state. Check its owner and permissions.")]
    Storage,
    #[error(
        "Keychain access failed. Unlock your login Keychain or explicitly choose --credential-store file."
    )]
    Keychain,
    #[error("Sign in privately with `ametller-mcp login` before accessing your account cart.")]
    LoginRequired,
    #[error("Stored session configuration changed. Sign in again with `ametller-mcp login`.")]
    SessionChanged,
    #[error("Another process is using this account. Retry after it finishes.")]
    Busy,
    #[error(
        "Cart write outcome is uncertain. Read get_cart and inspect Ametller's website; no automatic retry was made. Further writes are blocked until the desired state is observed or you explicitly run `cart --clear-uncertain` after checking the cart."
    )]
    Uncertain,
    #[error(
        "A previous cart write has an unresolved outcome. Read get_cart, inspect the cart, and use `cart --clear-uncertain` only after resolving it."
    )]
    Pending,
}

pub type Result<T> = std::result::Result<T, Error>;
