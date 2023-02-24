use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;
use void::Void;

pub fn string_or_struct<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'de> + FromStr<Err = Void>,
    D: Deserializer<'de>,
{
    // This is a Visitor that forwards string types to T's `FromStr` impl and
    // forwards map types to T's `Deserialize` impl. The `PhantomData` is to
    // keep the compiler from complaining about T being an unused generic type
    // parameter. We need T in order to know the Value type for the Visitor
    // impl.
    struct StringOrStruct<T>(PhantomData<fn() -> T>);

    impl<'de, T> Visitor<'de> for StringOrStruct<T>
    where
        T: Deserialize<'de> + FromStr<Err = Void>,
    {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("string or map")
        }

        fn visit_str<E>(self, value: &str) -> Result<T, E>
        where
            E: de::Error,
        {
            Ok(FromStr::from_str(value).unwrap())
        }

        fn visit_map<M>(self, map: M) -> Result<T, M::Error>
        where
            M: MapAccess<'de>,
        {
            // `MapAccessDeserializer` is a wrapper that turns a `MapAccess`
            // into a `Deserializer`, allowing it to be used as the input to T's
            // `Deserialize` implementation. T then deserializes itself using
            // the entries from the map visitor.
            Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_any(StringOrStruct(PhantomData))
}

pub fn optional_string_or_list<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrList;

    impl<'de> Visitor<'de> for StringOrList {
        type Value = Option<Vec<String>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("string or array")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(vec![String::from(value)]))
        }

        fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            Ok(Some(
                Deserialize::deserialize(de::value::SeqAccessDeserializer::new(seq)).unwrap(),
            ))
        }
    }

    deserializer.deserialize_any(StringOrList)
}

pub fn optional_list_of_strings_or_hash_of_strings_option_strings<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, Option<String>>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct ListOfStringsOrHashMap;

    impl<'de> Visitor<'de> for ListOfStringsOrHashMap {
        type Value = Option<HashMap<String, Option<String>>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("list or hashmap")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut result: HashMap<String, Option<String>> = HashMap::new();

            let list_of_vars: Vec<String> =
                Deserialize::deserialize(de::value::SeqAccessDeserializer::new(seq)).unwrap();

            for value in list_of_vars {
                let element: Vec<&str> = value.split('=').collect();
                if element.len() > 1 {
                    result.insert(element[0].to_string(), Some(element[1].to_string()));
                } else {
                    result.insert(element[0].to_string(), None);
                }
            }
            Ok(Some(result))
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

    deserializer.deserialize_any(ListOfStringsOrHashMap)
}

pub fn optional_list_of_numbers_as_strings<'de, T, D>(
    deserializer: D,
) -> Result<Option<Vec<T>>, D::Error>
where
    T: Deserialize<'de> + FromStr<Err = std::num::ParseIntError>,
    D: Deserializer<'de>,
{
    struct ListOfNumbers<T>(PhantomData<fn() -> T>);

    impl<'de, T> Visitor<'de> for ListOfNumbers<T>
    where
        T: Deserialize<'de> + FromStr<Err = std::num::ParseIntError>,
    {
        type Value = Option<Vec<T>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("list of numbers ")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let strings: Vec<String> =
                Deserialize::deserialize(de::value::SeqAccessDeserializer::new(seq)).unwrap();
            Ok(Some(
                strings
                    .into_iter()
                    .map(|value| value.parse::<T>().unwrap())
                    .collect::<Vec<T>>(),
            ))
        }
    }

    deserializer.deserialize_any(ListOfNumbers(PhantomData))
}
