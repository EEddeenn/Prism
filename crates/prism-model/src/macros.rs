//! Internal macros for declaring ID newtypes and bitflag sets without external deps.

/// Declare a transparent integer-newtype identifier with the standard derives,
/// constructors, and `Debug`/`Display` impls.
macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident($inner:ty)) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        #[repr(transparent)]
        pub struct $name(pub $inner);

        impl $name {
            #[doc = concat!("Construct a [`", stringify!($name), "`] from its raw value.")]
            #[inline]
            pub const fn new(v: $inner) -> Self {
                Self(v)
            }

            #[doc = concat!("Return the raw integer value of this [`", stringify!($name), "`].")]
            #[inline]
            pub const fn get(self) -> $inner {
                self.0
            }
        }

        impl ::core::convert::From<$inner> for $name {
            #[inline]
            fn from(v: $inner) -> Self {
                Self(v)
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

/// Declare a `Copy` bitflag set backed by a single unsigned integer, with no
/// external `bitflags` dependency. Generates named constants, set algebra, a
/// truncating constructor, and a `Debug` impl that lists the active flag names.
macro_rules! bitflag_set {
    (
        $(#[$meta:meta])*
        pub struct $name:ident: $ty:ty {
            $( const $flag:ident = $val:expr; )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
        #[repr(transparent)]
        pub struct $name($ty);

        impl $name {
            /// The empty set (no flags).
            pub const EMPTY: $name = $name(0);
            /// The union of every named flag in this set.
            pub const ALL: $name = $name(0 $( | $val )*);

            $( pub const $flag: $name = $name($val); )*

            /// Raw bit pattern.
            #[inline]
            pub const fn bits(self) -> $ty {
                self.0
            }

            /// Construct from raw bits, discarding any bit not part of a named flag.
            #[inline]
            pub const fn from_bits_truncate(bits: $ty) -> Self {
                $name(bits & Self::ALL.0)
            }

            /// True if `self` is empty.
            #[inline]
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            /// True if every flag in `other` is present in `self`.
            #[inline]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            /// True if `self` and `other` share any flag.
            #[inline]
            pub const fn intersects(self, other: Self) -> bool {
                self.0 & other.0 != 0
            }

            /// Add the flags in `other`.
            #[inline]
            pub fn insert(&mut self, other: Self) {
                self.0 |= other.0;
            }

            /// Remove the flags in `other`.
            #[inline]
            pub fn remove(&mut self, other: Self) {
                self.0 &= !other.0;
            }

            /// Named flags in declaration order, with their bit values.
            pub const NAMED: &'static [(&'static str, $ty)] = &[
                $( (stringify!($flag), $val), )*
            ];
        }

        impl ::core::ops::BitOr for $name {
            type Output = $name;
            #[inline]
            fn bitor(self, rhs: Self) -> Self {
                $name(self.0 | rhs.0)
            }
        }

        impl ::core::ops::BitAnd for $name {
            type Output = $name;
            #[inline]
            fn bitand(self, rhs: Self) -> Self {
                $name(self.0 & rhs.0)
            }
        }

        impl ::core::ops::BitOrAssign for $name {
            #[inline]
            fn bitor_assign(&mut self, rhs: Self) {
                self.0 |= rhs.0;
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(f, "{}(", stringify!($name))?;
                let mut first = true;
                for (name, val) in Self::NAMED {
                    if self.0 & val == *val && *val != 0 {
                        if !first {
                            write!(f, " | ")?;
                        }
                        write!(f, "{name}")?;
                        first = false;
                    }
                }
                if first {
                    write!(f, "EMPTY")?;
                }
                write!(f, ")")
            }
        }
    };
}
