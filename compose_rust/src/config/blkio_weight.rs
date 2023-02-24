use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct BlkioWeight {
    pub path: Option<String>,
    pub weight: Option<i32>,
}
