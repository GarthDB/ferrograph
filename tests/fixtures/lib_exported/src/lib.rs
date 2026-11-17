pub mod open;
mod hidden;
mod reexp;
mod globbed;
mod chain_a;
mod chain_b;
pub(crate) mod crate_mod;
mod renamed;
// Both cfg variants are indexed whatever the platform; the expected dead list in the pipeline test relies on that.
#[cfg(unix)]
mod platform;
#[cfg(not(unix))]
#[path = "platform_other.rs"]
mod platform;
#[doc(hidden)]
pub mod hidden_api;
#[path = "weird.rs"]
mod pathmod;

pub use chain_a::chained_fn;
pub use crate_mod::via_crate_mod;
pub use globbed::*;
pub use reexp::exported_fn;
pub use renamed::original as alias;
pub use platform::shared;
// Does not resolve (no such module): every fn named `ext_fn` stays exported (hidden.rs), the documented fallback.
pub use external_crate::ext_fn;

pub fn root_fn() {}
pub use reexp::EXPORTED_CONST;
