#[macro_export]
macro_rules! inline_if {
    ($condition:expr, $true_expr:expr, $false_expr:expr) => {
        if $condition { $true_expr } else { $false_expr }
    };
}

#[macro_export]
macro_rules! const_assert_eq {
    ($left:expr, $right:expr $(,)?) => {
        const _: () = assert!($left == $right);
    };
}

#[macro_export]
macro_rules! const_assert {
    ($($tt:tt)*) => {
        const _: () = assert!($($tt)*);
    }
}

pub trait NumberUtils<T> {
    fn prev_power_of_two(self) -> T;
}

impl NumberUtils<usize> for usize {
    fn prev_power_of_two(self) -> usize {
        1 << (usize::BITS as usize - self.leading_zeros() as usize - 1)
    }
}
