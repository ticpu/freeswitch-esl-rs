//! Internal declarative macros shared across the crate.
//!
//! These are `#[macro_use]`-imported from `lib.rs` and are not part of the
//! public API. They exist to reduce mechanical repetition in types that are
//! "wire-string ↔ enum" mappings (see `channel.rs`, `sofia/`).

/// Generate a single-field parse-error newtype.
///
/// Expands to `pub struct $Error(pub String)`, a `Display` impl emitting
/// `"unknown <label>: <value>"`, and `impl std::error::Error`.
///
/// ```ignore
/// parse_error! { ParseFooError("foo"); }
/// ```
macro_rules! parse_error {
    ($Error:ident($label:literal);) => {
        #[doc = concat!("Error returned when parsing an invalid ", $label, " string.")]
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $Error(pub String);

        impl ::std::fmt::Display for $Error {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, concat!("unknown ", $label, ": {}"), self.0)
            }
        }

        impl ::std::error::Error for $Error {}
    };
}

/// Generate an enum with canonical wire-string mappings and a matching
/// `Parse<Name>Error` error type.
///
/// Expands to the enum itself, `as_str()`, `Display`, a `ParseFooError`
/// newtype (`Debug + Clone + PartialEq + Eq + std::error::Error`), and
/// `FromStr` that accepts the canonical case only.
///
/// Input shape:
///
/// ```ignore
/// wire_enum! {
///     /// Doc comment on the enum.
///     #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
///     pub enum CallDirection {
///         /// Optional per-variant attrs (docs, cfg_attr).
///         Inbound => "inbound",
///         Outbound => "outbound",
///     }
///     error ParseCallDirectionError("call direction");
/// }
/// ```
///
/// The caller supplies the derive list and any additional attributes (repr,
/// cfg_attr, allow). The macro appends `#[non_exhaustive]`,
/// `#[allow(missing_docs)]`, and `#[cfg_attr(feature = "serde", derive(...))]`
/// so those do not need to be duplicated at the call sites.
///
/// An optional trailing `tests: <module_ident>;` clause emits a
/// `#[cfg(test)] mod <module_ident>` containing exhaustive round-trip,
/// wrong-case-rejection, and unknown-rejection tests. The module name must
/// be unique within the enclosing scope.
///
/// An optional `numeric: <fn_name>(<repr>);` clause emits `from_number(n: <repr>)
/// -> Option<Self>` and `as_number(&self) -> <repr>` based on the per-variant
/// discriminants. Every variant must declare a discriminant (`= NUM`) when
/// this clause is used.
macro_rules! wire_enum {
    // With tests + numeric: emit base + numeric + test module.
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $Enum:ident {
            $(
                $(#[$var_meta:meta])*
                $Variant:ident = $disc:literal => $wire:literal
            ),+ $(,)?
        }
        error $Error:ident($label:literal);
        numeric: $numeric_fn:ident($repr:ty);
        tests: $tests_mod:ident;
    ) => {
        wire_enum! {
            $(#[$enum_meta])*
            $vis enum $Enum {
                $(
                    $(#[$var_meta])*
                    $Variant = $disc => $wire
                ),+
            }
            error $Error($label);
            numeric: $numeric_fn($repr);
        }

        wire_enum! { @tests $Enum, $Error, $label, $tests_mod, $($Variant => $wire),+ }
    };

    // Internal: emit the standard test module.
    (
        @tests $Enum:ident, $Error:ident, $label:literal, $tests_mod:ident,
        $($Variant:ident => $wire:literal),+
    ) => {
        #[cfg(test)]
        mod $tests_mod {
            use super::*;

            #[test]
            fn display_and_from_str_roundtrip() {
                $(
                    assert_eq!($Enum::$Variant.to_string(), $wire);
                    assert_eq!($wire.parse::<$Enum>(), Ok($Enum::$Variant));
                )+
            }

            #[test]
            fn from_str_rejects_wrong_case() {
                $({
                    let wire: &str = $wire;
                    let lower = wire.to_ascii_lowercase();
                    if lower != wire {
                        assert!(
                            lower.parse::<$Enum>().is_err(),
                            concat!(
                                stringify!($Enum),
                                " must reject lowercased \"",
                                $wire,
                                "\"",
                            ),
                        );
                    }
                    let upper = wire.to_ascii_uppercase();
                    if upper != wire {
                        assert!(
                            upper.parse::<$Enum>().is_err(),
                            concat!(
                                stringify!($Enum),
                                " must reject uppercased \"",
                                $wire,
                                "\"",
                            ),
                        );
                    }
                })+
            }

            #[test]
            fn from_str_rejects_unknown() {
                let err = "__wire_enum_bogus_sentinel__".parse::<$Enum>().unwrap_err();
                assert_eq!(err.0, "__wire_enum_bogus_sentinel__");
                assert!(err.to_string().contains($label));
            }
        }
    };

    // Numeric only, no tests.
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $Enum:ident {
            $(
                $(#[$var_meta:meta])*
                $Variant:ident = $disc:literal => $wire:literal
            ),+ $(,)?
        }
        error $Error:ident($label:literal);
        numeric: $numeric_fn:ident($repr:ty);
    ) => {
        wire_enum! {
            $(#[$enum_meta])*
            $vis enum $Enum {
                $(
                    $(#[$var_meta])*
                    $Variant = $disc => $wire
                ),+
            }
            error $Error($label);
        }

        impl $Enum {
            /// Look up by numeric discriminant.
            pub fn $numeric_fn(n: $repr) -> Option<Self> {
                match n {
                    $( $disc => Some(Self::$Variant), )+
                    _ => None,
                }
            }

            /// Numeric discriminant value.
            pub fn as_number(&self) -> $repr {
                *self as $repr
            }
        }
    };

    // With tests: emit the base expansion then the test module.
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $Enum:ident {
            $(
                $(#[$var_meta:meta])*
                $Variant:ident $(= $disc:literal)? => $wire:literal
            ),+ $(,)?
        }
        error $Error:ident($label:literal);
        tests: $tests_mod:ident;
    ) => {
        wire_enum! {
            $(#[$enum_meta])*
            $vis enum $Enum {
                $(
                    $(#[$var_meta])*
                    $Variant $(= $disc)? => $wire
                ),+
            }
            error $Error($label);
        }

        wire_enum! { @tests $Enum, $Error, $label, $tests_mod, $($Variant => $wire),+ }
    };

    // Base expansion, no tests.
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $Enum:ident {
            $(
                $(#[$var_meta:meta])*
                $Variant:ident $(= $disc:literal)? => $wire:literal
            ),+ $(,)?
        }
        error $Error:ident($label:literal);
    ) => {
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        $(#[$enum_meta])*
        #[non_exhaustive]
        #[allow(missing_docs)]
        $vis enum $Enum {
            $(
                $(#[$var_meta])*
                $Variant $(= $disc)?,
            )+
        }

        impl $Enum {
            /// Canonical wire-format string.
            pub const fn as_str(&self) -> &'static str {
                match self {
                    $( Self::$Variant => $wire, )+
                }
            }
        }

        impl ::std::fmt::Display for $Enum {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $Enum {
            type Err = $Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $wire => Ok(Self::$Variant), )+
                    _ => Err($Error(s.to_string())),
                }
            }
        }

        parse_error! { $Error($label); }
    };
}
