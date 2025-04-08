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
            #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
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
        )*
    };
}
