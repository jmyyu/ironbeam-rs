//! Shared building blocks: the lenient wire-enum macro, serde helpers and the
//! base response envelope.

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

/// Defines a string-backed enum that never fails to deserialize.
///
/// * The first literal is the canonical wire form used when serializing.
/// * Further literals are aliases accepted on input (the specification is
///   inconsistent about codes vs. names, see `docs/SPEC_DISCREPANCIES.md`).
/// * Matching is ASCII case-insensitive and also accepts JSON numbers.
/// * Anything else is preserved in `Unknown(raw)`.
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $canon:literal $(| $alias:literal)* ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant, )+
            /// A value unknown to this SDK version; the raw wire value is kept.
            Unknown(String),
        }

        impl $name {
            /// Canonical wire representation.
            pub fn as_wire(&self) -> &str {
                match self {
                    $( Self::$variant => $canon, )+
                    Self::Unknown(raw) => raw,
                }
            }

            /// Parses a wire value; never fails.
            pub fn from_wire(raw: &str) -> Self {
                $(
                    if raw.eq_ignore_ascii_case($canon) $(|| raw.eq_ignore_ascii_case($alias))* {
                        return Self::$variant;
                    }
                )+
                Self::Unknown(raw.to_owned())
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_wire())
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_wire())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> ::serde::de::Visitor<'de> for V {
                    type Value = $name;
                    fn expecting(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                        f.write_str("a string or number")
                    }
                    fn visit_str<E: ::serde::de::Error>(self, v: &str) -> Result<$name, E> {
                        Ok($name::from_wire(v))
                    }
                    fn visit_i64<E: ::serde::de::Error>(self, v: i64) -> Result<$name, E> {
                        Ok($name::from_wire(&v.to_string()))
                    }
                    fn visit_u64<E: ::serde::de::Error>(self, v: u64) -> Result<$name, E> {
                        Ok($name::from_wire(&v.to_string()))
                    }
                    fn visit_f64<E: ::serde::de::Error>(self, v: f64) -> Result<$name, E> {
                        Ok($name::from_wire(&v.to_string()))
                    }
                    fn visit_bool<E: ::serde::de::Error>(self, v: bool) -> Result<$name, E> {
                        Ok($name::from_wire(if v { "true" } else { "false" }))
                    }
                }
                d.deserialize_any(V)
            }
        }
    };
}
pub(crate) use wire_enum;

/// Deserializes `null` as `T::default()` (used for arrays the server may send
/// as `null`).
pub(crate) fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// Accepts a single object, an array of objects, or `null`, always yielding a
/// `Vec`. Several stream fields are documented as a single object in one place
/// and as a list in another.
pub(crate) fn one_or_many<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        Many(Vec<T>),
        One(T),
    }
    Ok(match Option::<OneOrMany<T>>::deserialize(d)? {
        None => Vec::new(),
        Some(OneOrMany::Many(v)) => v,
        Some(OneOrMany::One(x)) => vec![x],
    })
}

/// Accepts `[T, ...]` or `[[T, ...], ...]` (flattened) or `null`.
pub(crate) fn flat_or_nested<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Item<T> {
        Nested(Vec<T>),
        Flat(T),
    }
    let items = Option::<Vec<Item<T>>>::deserialize(d)?.unwrap_or_default();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Item::Nested(v) => out.extend(v),
            Item::Flat(x) => out.push(x),
        }
    }
    Ok(out)
}

/// Accepts an identifier sent either as a JSON string or as a JSON number.
pub(crate) fn string_or_number<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        S(String),
        I(i64),
        U(u64),
        F(f64),
    }
    Ok(match Raw::deserialize(d)? {
        Raw::S(s) => s,
        Raw::I(n) => n.to_string(),
        Raw::U(n) => n.to_string(),
        Raw::F(n) => n.to_string(),
    })
}

/// Optional variant of [`string_or_number`].
pub(crate) fn opt_string_or_number<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    struct Wrap(#[serde(deserialize_with = "string_or_number")] String);
    Ok(Option::<Wrap>::deserialize(d)?.map(|w| w.0))
}

/// Accepts an integer sent as a JSON integer, an integral float, or a numeric
/// string.
pub(crate) fn opt_lenient_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        I(i64),
        F(f64),
        S(String),
    }
    match Option::<Raw>::deserialize(d)? {
        None => Ok(None),
        Some(Raw::I(n)) => Ok(Some(n)),
        Some(Raw::F(f)) if f.fract() == 0.0 && f.abs() < 9.0e15 => Ok(Some(f as i64)),
        Some(Raw::F(f)) => Err(de::Error::custom(format!("expected an integer, got {f}"))),
        Some(Raw::S(s)) if s.trim().is_empty() => Ok(None),
        Some(Raw::S(s)) => s
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| de::Error::custom(format!("expected an integer, got {s:?}"))),
    }
}

wire_enum! {
    /// `status` field of the base response envelope.
    pub enum ResponseStatus {
        Ok = "OK",
        Error = "ERROR",
        Warning = "WARNING",
        Info = "INFO",
        Fatal = "FATAL",
        UnknownStatus = "UNKNOWN",
    }
}

impl ResponseStatus {
    /// `true` for `ERROR` and `FATAL`.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Error | Self::Fatal)
    }
}

/// Base response envelope (`Response` schema): a status and a message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiResponse {
    /// Outcome reported by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ResponseStatus>,
    /// Human-readable message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
