//! The values the lobby chose for the settings a game version declares in `mugon.toml`.
//!
//! They are fixed when the game starts, so [`Bridge`](crate::Bridge) reads them once. Whether a
//! setting may be edited (`enabled-when`) or the game may start (`settings-valid-when`) is
//! enforced by the platform before the game launches; the game only ever sees the final values.

use js_sys::{Array, Object};
use std::collections::BTreeMap;
use wasm_bindgen::JsValue;

/// A setting value, carrying the type the setting was declared with.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    /// A `boolean` setting.
    Bool(bool),
    /// A `number` setting. Every number setting is an `f64`, whatever its `step`.
    Number(f64),
    /// A `text` setting, or a single-choice `select`.
    Text(String),
    /// A `select` setting with `multiple = true`.
    List(Vec<String>),
}

impl SettingValue {
    fn to_js(&self) -> JsValue {
        match self {
            Self::Bool(boolean) => JsValue::from_bool(*boolean),
            Self::Number(number) => JsValue::from_f64(*number),
            Self::Text(text) => JsValue::from_str(text),
            Self::List(items) => items
                .iter()
                .map(|item| JsValue::from_str(item))
                .collect::<Array>()
                .into(),
        }
    }

    fn from_js(value: &JsValue) -> Option<Self> {
        if let Some(boolean) = value.as_bool() {
            return Some(Self::Bool(boolean));
        }
        if let Some(number) = value.as_f64() {
            return Some(Self::Number(number));
        }
        if let Some(text) = value.as_string() {
            return Some(Self::Text(text));
        }
        if Array::is_array(value) {
            let items = Array::from(value)
                .iter()
                .map(|item| item.as_string())
                .collect::<Option<Vec<_>>>()?;
            return Some(Self::List(items));
        }
        None
    }
}

pub(crate) fn write_settings(settings: &BTreeMap<String, SettingValue>) -> Object {
    let object = Object::new();
    for (key, value) in settings {
        // Setting a plain property on a fresh object cannot fail.
        let _ = js_sys::Reflect::set(&object, &JsValue::from_str(key), &value.to_js());
    }
    object
}

pub(crate) fn read_settings(settings: &Object) -> BTreeMap<String, SettingValue> {
    Object::entries(settings)
        .iter()
        .filter_map(|entry| {
            let entry = Array::from(&entry);
            let key = entry.get(0).as_string()?;
            let value = SettingValue::from_js(&entry.get(1))?;
            Some((key, value))
        })
        .collect()
}

#[cfg(feature = "serde")]
pub use deserialize::SettingsError;

#[cfg(feature = "serde")]
pub(crate) use deserialize::from_settings;

#[cfg(feature = "serde")]
mod deserialize {
    use super::SettingValue;
    use serde::Deserialize;
    use serde::de::value::{Error, MapDeserializer, SeqDeserializer};
    use serde::de::{Deserializer, IntoDeserializer, Visitor};
    use std::collections::BTreeMap;

    /// Why the settings don't fit the requested struct: a missing key, or a value of another
    /// type than the field expects. The message names the offending field.
    pub type SettingsError = Error;

    struct ValueDeserializer(SettingValue);

    struct Value(SettingValue);

    impl<'de> IntoDeserializer<'de, Error> for Value {
        type Deserializer = ValueDeserializer;

        fn into_deserializer(self) -> Self::Deserializer {
            ValueDeserializer(self.0)
        }
    }

    impl<'de> Deserializer<'de> for ValueDeserializer {
        type Error = Error;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self.0 {
                SettingValue::Bool(boolean) => visitor.visit_bool(boolean),
                // Integral values go through the integer visitors, so a `u32` or `i64` field
                // accepts a whole-numbered setting; the float visitors accept integers too.
                SettingValue::Number(number) if number.fract() == 0.0 && number >= 0.0 => {
                    visitor.visit_u64(number as u64)
                }
                SettingValue::Number(number) if number.fract() == 0.0 => {
                    visitor.visit_i64(number as i64)
                }
                SettingValue::Number(number) => visitor.visit_f64(number),
                SettingValue::Text(text) => visitor.visit_string(text),
                SettingValue::List(items) => {
                    visitor.visit_seq(SeqDeserializer::new(items.into_iter()))
                }
            }
        }

        fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            visitor.visit_some(self)
        }

        fn deserialize_enum<V: Visitor<'de>>(
            self,
            _name: &'static str,
            _variants: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Error> {
            match self.0 {
                SettingValue::Text(text) => visitor.visit_enum(text.into_deserializer()),
                other => ValueDeserializer(other).deserialize_any(visitor),
            }
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
            byte_buf unit unit_struct newtype_struct seq tuple tuple_struct map struct
            identifier ignored_any
        }
    }

    pub(crate) fn from_settings<'de, T: Deserialize<'de>>(
        settings: &BTreeMap<String, SettingValue>,
    ) -> Result<T, SettingsError> {
        T::deserialize(MapDeserializer::new(
            settings
                .iter()
                .map(|(key, value)| (key.as_str(), Value(value.clone()))),
        ))
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "kebab-case")]
    enum Map {
        Desert,
        Forest,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct GameSettings {
        #[serde(rename = "friendly-fire")]
        friendly_fire: bool,
        #[serde(rename = "round-time")]
        round_time: u32,
        gravity: f32,
        map: Map,
        modifiers: Vec<String>,
        #[serde(rename = "server.name")]
        server_name: String,
        #[serde(default)]
        bots: Option<bool>,
    }

    fn settings() -> BTreeMap<String, SettingValue> {
        BTreeMap::from([
            ("friendly-fire".into(), SettingValue::Bool(true)),
            ("round-time".into(), SettingValue::Number(180.0)),
            ("gravity".into(), SettingValue::Number(9.5)),
            ("map".into(), SettingValue::Text("forest".into())),
            (
                "modifiers".into(),
                SettingValue::List(vec!["fast".into(), "low_gravity".into()]),
            ),
            ("server.name".into(), SettingValue::Text("My server".into())),
        ])
    }

    #[test]
    fn deserializes_every_value_kind() {
        let parsed: GameSettings = from_settings(&settings()).unwrap();
        assert_eq!(
            parsed,
            GameSettings {
                friendly_fire: true,
                round_time: 180,
                gravity: 9.5,
                map: Map::Forest,
                modifiers: vec!["fast".into(), "low_gravity".into()],
                server_name: "My server".into(),
                bots: None,
            }
        );
    }

    #[test]
    fn rejects_a_value_of_the_wrong_type() {
        let mut values = settings();
        values.insert("round-time".into(), SettingValue::Text("long".into()));
        let error = from_settings::<GameSettings>(&values).unwrap_err();
        assert!(error.to_string().contains("expected u32"), "{error}");
    }

    #[test]
    fn rejects_a_fractional_number_for_an_integer_field() {
        let mut values = settings();
        values.insert("round-time".into(), SettingValue::Number(1.5));
        assert!(from_settings::<GameSettings>(&values).is_err());
    }

    #[test]
    fn reports_a_missing_key() {
        let mut values = settings();
        values.remove("map");
        let error = from_settings::<GameSettings>(&values).unwrap_err();
        assert!(error.to_string().contains("map"), "{error}");
    }

    #[test]
    fn single_choice_select_reads_as_text() {
        let parsed: GameSettings = from_settings(&settings()).unwrap();
        assert_eq!(parsed.map, Map::Forest);
    }
}
