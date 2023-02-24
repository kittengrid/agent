use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct Healthcheck {
    pub disable: bool,
    pub interval: Option<String>,
    pub retries: Option<f32>,
    //    #[serde(deserialize_with = "super::build::optional_string_or_test")]
    //    pub test: Option<Vec<String>>,
    pub timeout: Option<String>,
    pub start_period: Option<String>,
}
