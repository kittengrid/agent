use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;
use void::Void;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct SecretRef {
    pub source: String,
    pub target: Option<String>,
    pub uid: Option<String>,
    pub gid: Option<String>,
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_mode() -> String {
    String::from("0444")
}
#[derive(Debug, Deserialize)]
pub struct WrappedSecretRef(
    #[serde(deserialize_with = "super::utils::string_or_struct")] SecretRef,
);

impl FromStr for SecretRef {
    // This implementation of `from_str` can never fail, so use the impossible
    // `Void` type as the error type.
    type Err = Void;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(SecretRef {
            source: s.to_string(),
            target: None,
            uid: None,
            gid: None,
            mode: String::from("0444"),
        })
    }
}

// @TODO: Make this generic (also used in congig)
pub fn optional_array_of_strings_or_secret_refs<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<SecretRef>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Vec<WrappedSecretRef>>::deserialize(deserializer).map(
        |opt_wrapped: Option<Vec<WrappedSecretRef>>| {
            opt_wrapped.map(|array_of_wrapped_secret_refs: Vec<WrappedSecretRef>| {
                array_of_wrapped_secret_refs
                    .into_iter()
                    .map(|wrapped_secret_ref: WrappedSecretRef| wrapped_secret_ref.0)
                    .collect::<Vec<SecretRef>>()
            })
        },
    )
}
