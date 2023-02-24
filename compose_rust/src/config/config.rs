use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;
use void::Void;

#[derive(Serialize, Deserialize, Debug)]
struct OptionalConfig(Option<Config>);

#[derive(Debug, Deserialize)]
pub struct WrappedConfig(#[serde(deserialize_with = "super::utils::string_or_struct")] Config);

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct Config {
    source: String,
    target: Option<String>,
    uid: Option<String>,
    gid: Option<String>,
    mode: Option<i32>,
}

impl FromStr for Config {
    // This implementation of `from_str` can never fail, so use the impossible
    // `Void` type as the error type.
    type Err = Void;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Config {
            source: s.to_string(),
            target: None,
            uid: None,
            gid: None,
            mode: None,
        })
    }
}

pub fn optional_array_of_strings_or_configs<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<Config>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Vec<WrappedConfig>>::deserialize(deserializer).map(
        |opt_wrapped: Option<Vec<WrappedConfig>>| {
            opt_wrapped.map(|array_of_wrapped_configs: Vec<WrappedConfig>| {
                array_of_wrapped_configs
                    .into_iter()
                    .map(|wrapped_config: WrappedConfig| wrapped_config.0)
                    .collect::<Vec<Config>>()
            })
        },
    )
}
