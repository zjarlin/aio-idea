pub mod model;

#[cfg(any(feature = "web", feature = "desktop"))]
pub mod client;
#[cfg(any(feature = "web", feature = "desktop"))]
mod frontend;
#[cfg(feature = "server")]
pub mod server;

pub use model::*;
