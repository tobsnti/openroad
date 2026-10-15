pub(crate) mod archive;
pub(crate) mod blowfish;
pub(crate) mod constants;
pub(crate) mod directory;
pub(crate) mod entry;
pub(crate) mod errors;
pub(crate) mod header;
pub(crate) mod key;
pub(crate) mod read_stats;
pub(crate) mod source;
/// Reading the archive over the network instead of from a file. Off by
/// default: it is the only thing in this crate that needs an HTTP client, and
/// never built for wasm32, where that client has no build (see `Cargo.toml`).
#[cfg(all(feature = "http-source", not(target_arch = "wasm32")))]
pub mod source_http;
pub(crate) mod util;
