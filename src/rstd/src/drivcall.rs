use core::arch::asm;
use pager::address::VirtAddr;

pub const DRIVCALL_ERR_VSYSCALL_FULL: u64 = 1 << 10;

pub const DRIVCALL_SPAWN: u64 = 1;
pub const DRIVCALL_SLEEP: u64 = 2;
pub const DRIVCALL_EXIT: u64 = 3;
pub const DRIVCALL_FUTEX_WAIT: u64 = 4;
pub const DRIVCALL_FUTEX_WAKE: u64 = 5;
pub const DRIVCALL_VSYS_REG: u64 = 6;
pub const DRIVCALL_VSYS_WAIT: u64 = 7;
pub const DRIVCALL_VSYS_REQ: u64 = 8;
pub const DRIVCALL_VSYS_RET: u64 = 9;
pub const DRIVCALL_INT_WAIT: u64 = 10;
pub const DRIVCALL_PIN: u64 = 11;
pub const DRIVCALL_UNPIN: u64 = 12;
pub const DRIVCALL_ISPIN: u64 = 13;
pub const DRIVCALL_THREAD_WAIT_EXIT: u64 = 14;
