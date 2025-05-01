#![no_std]

#[macro_export]
macro_rules! c_enum {
    (
        $(
            $(#[$meta:meta])*
            $vis:vis enum $name:ident: $type:ty {
                $(
                    $element_name:ident = $expr:expr
                )*
            }
        )*
    ) => {
        $(
            #[repr(transparent)]
            #[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
            $vis struct $name($type);

            #[allow(unused)]
            impl $name {
                $(
                    #[allow(non_upper_case_globals)]
                    pub const $element_name: $name = $name($expr);
                )*
            }

            impl From<$name> for $type {
                fn from(value: $name) -> Self {
                    value.0
                }
            }

            impl core::fmt::Debug for $name {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    let value = match self {
                        $(
                            $name(a) if *a == $expr => stringify!($element_name),
                        )*
                        _ => unreachable!()
                    };
                    write!(f, "{name}({value})", name = stringify!($name))
                }
            }
        )*
    };
}
