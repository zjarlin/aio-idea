use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ApplicationSnapshot {
    pub permissions: Vec<String>,
    pub catalog: crate::runtime::RuntimeCatalog,
}
