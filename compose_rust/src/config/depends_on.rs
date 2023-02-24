use indexmap::IndexMap;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use std::fmt;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum DependsOnCondition {
    #[serde(alias = "service_healthy")]
    ServiceHealthy,
    #[serde(alias = "service_started")]
    ServiceStarted,
    #[serde(alias = "service_completed_successfully")]
    ServiceCompletedSuccessfully,
}
impl Default for DependsOnCondition {
    fn default() -> Self {
        DependsOnCondition::ServiceStarted
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct DependsOn {
    condition: DependsOnCondition,
}

pub fn optional_array_of_strings_or_ordered_hash_of_structs<'de, D>(
    deserializer: D,
) -> Result<Option<IndexMap<String, DependsOn>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct DependsOnInput;

    impl<'de> Visitor<'de> for DependsOnInput {
        type Value = Option<IndexMap<String, DependsOn>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("strings or maps")
        }

        fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let result: Vec<String> =
                Deserialize::deserialize(de::value::SeqAccessDeserializer::new(seq)).unwrap();

            let mut result_map: IndexMap<String, DependsOn> = IndexMap::new();

            for value in result {
                result_map.insert(
                    value.to_string(),
                    DependsOn {
                        condition: DependsOnCondition::ServiceStarted,
                    },
                );
            }

            Ok(Some(result_map))
        }

        fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            Ok(Some(
                Deserialize::deserialize(de::value::MapAccessDeserializer::new(map)).unwrap(),
            ))
        }
    }

    deserializer.deserialize_any(DependsOnInput)
}
