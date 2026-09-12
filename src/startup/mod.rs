mod model;
pub(crate) use model::ApplicationSnapshot;

#[cfg(any(feature = "web", feature = "desktop"))]
mod client;
#[cfg(any(feature = "web", feature = "desktop"))]
pub(crate) use client::{LoadedApplication, load};
