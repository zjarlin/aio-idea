mod controller;
mod discovery;
mod documents;
mod rollout;
#[cfg(test)]
mod rollout_tests;
mod store;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(super) use rollout_tests::exercise_rollouts;

pub(super) use controller::router;
pub(super) use store::migrate;

pub(super) fn start(state: super::RuntimeState) {
    if std::env::var("AIO_DELIVERY_TOKEN").is_ok_and(|s| !s.is_empty()) {
        tokio::spawn(discovery::run(state.clone()));
        tokio::spawn(rollout::run(state));
    }
}

pub(super) use rollout::{ensure_current, exclude_revision, remember_installation};
