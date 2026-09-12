mod controller;
mod discovery;
mod documents;
mod rollout;
mod store;
#[cfg(test)]
mod tests;

pub(super) use controller::router;
pub(super) use store::migrate;

pub(super) fn start(state: super::RuntimeState) {
    if std::env::var("AIO_DELIVERY_TOKEN").is_ok_and(|s| !s.is_empty()) {
        tokio::spawn(discovery::run(state.clone()));
        tokio::spawn(rollout::run(state));
    }
}

pub(super) use rollout::{ensure_current, exclude_revision, remember_installation};
