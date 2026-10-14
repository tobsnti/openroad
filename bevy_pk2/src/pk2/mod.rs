pub(crate) mod archive;
pub(crate) mod blowfish;
pub(crate) mod constants;
pub(crate) mod directory;
pub(crate) mod entry;
pub(crate) mod errors;
pub(crate) mod header;
pub(crate) mod key;
/// Resolving one path by walking only the directories on it, instead of
/// indexing the whole archive first.
pub(crate) mod lazy_index;
pub(crate) mod read_stats;
pub(crate) mod source;
/// Reading the archive over the network instead of from a file. Off by
/// default: it is the only thing in this crate that needs an HTTP client.
#[cfg(feature = "http-source")]
pub mod source_http;
pub(crate) mod util;
