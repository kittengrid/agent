use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct BlkioLimit {
    pub path: Option<String>,
    pub rate: Option<i32>,
}
