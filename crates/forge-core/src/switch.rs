//! An optional setting in a settings file (#211, D-053): `false` when it is off, its value
//! when it is on. Use it on an `Option` field with `#[serde(with = "forge_core::switch")]`.
//!
//! ```toml
//! [rivers]
//! brooks = false          # off
//! steps = { from = 0.02 } # on, with its settings
//! ```

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Writes `None` as `false` and `Some(value)` as the value.
pub fn serialize<S: Serializer, T: Serialize>(
    value: &Option<T>,
    out: S,
) -> Result<S::Ok, S::Error> {
    match value {
        None => out.serialize_bool(false),
        Some(value) => value.serialize(out),
    }
}

/// Reads `false` as `None` and anything else as `Some` of the value.
pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    input: D,
) -> Result<Option<T>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either<T> {
        Off(bool),
        On(T),
    }
    match Either::<T>::deserialize(input).map_err(|_| {
        D::Error::custom(format!(
            "expected `false` (off) or a {} (on); check its keys and their types",
            std::any::type_name::<T>()
                .rsplit("::")
                .next()
                .unwrap_or("value")
        ))
    })? {
        Either::Off(false) => Ok(None),
        Either::Off(true) => Err(D::Error::custom(
            "`true` is not a setting: give its value or table to turn it on",
        )),
        Either::On(value) => Ok(Some(value)),
    }
}
