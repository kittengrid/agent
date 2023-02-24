use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct CredentialSpec {
    pub config: Option<String>,
    pub file: Option<String>,
    pub registry: Option<String>,
}
