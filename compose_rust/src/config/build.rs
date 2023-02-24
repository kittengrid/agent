use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;
use void::Void;

#[derive(Serialize, Deserialize, Debug)]
struct OptionalBuild(Option<Build>);

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct Build {
    context: Option<String>,
    dockerfile: Option<String>,
    #[serde(deserialize_with = "super::utils::optional_string_or_list")]
    test: Option<Vec<String>>,
}
#[derive(Debug, Deserialize)]
pub struct WrappedBuild(#[serde(deserialize_with = "super::utils::string_or_struct")] Build);

impl FromStr for Build {
    // This implementation of `from_str` can never fail, so use the impossible
    // `Void` type as the error type.
    type Err = Void;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Build {
            context: Some(s.to_string()),
            dockerfile: None,
            test: None,
        })
    }
}

pub fn optional_string_or_build<'de, D>(
    deserializer: D,
) -> Result<Option<super::build::Build>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<WrappedBuild>::deserialize(deserializer)
        .map(|opt_wrapped: Option<WrappedBuild>| opt_wrapped.map(|wrapped: WrappedBuild| wrapped.0))
}
