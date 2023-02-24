use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct Logging {
    pub driver: Option<String>,
    pub options: Option<HashMap<String, String>>,
}
