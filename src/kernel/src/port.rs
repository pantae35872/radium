use core::{
    fmt::{Debug, LowerHex},
    marker::PhantomData,
};

use bit_field::BitField;

use crate::initialization_context::select_context;

const X86_PORT_SIZE: usize = 0xFFFF; // X86 port size is 16 bit
const X86_PORT_BITMAP_SIZE: usize = 2 * X86_PORT_SIZE / 8 + 1; // 2 Bits per port, R/W

pub struct PortAllocator {
    bitmap: [u8; X86_PORT_BITMAP_SIZE],
}

pub trait PortSize: Debug + PartialEq + Eq {
    type Type;

    fn size_in_bytes() -> usize;

    /// Reading to an unallocated port is unsafe considered using the [`PORT_ALLOCATOR`]
    unsafe fn read(address: u16) -> Self::Type;

    /// Writing to an unallocated port is unsafe considered using the [`PORT_ALLOCATOR`]
    unsafe fn write(address: u16, value: Self::Type);
}

macro_rules! port_size_impl {
    ($name:ident($type:ty): $reg:tt) => {
        #[derive(Debug, Eq, PartialEq)]
        pub struct $name;

        impl PortSize for $name {
            type Type = $type;

            fn size_in_bytes() -> usize {
                size_of::<$type>()
            }

            unsafe fn read(address: u16) -> Self::Type {
                let result: $type;
                unsafe {
                    core::arch::asm!(
                        concat!("in ", $reg, ", dx"),
                        in("dx") address,
                        out($reg) result,
                    )
                };
                result
            }

            unsafe fn write(address: u16, value: Self::Type) { unsafe {
                 core::arch::asm!(
                     concat!("out ", "dx, ", $reg),
                     in("dx") address,
                     in($reg) value,
                 )
            } }
        }
    };
}

port_size_impl!(Port8Bit(u8): "al");
port_size_impl!(Port16Bit(u16): "ax");
port_size_impl!(Port32Bit(u32): "eax");

pub trait PortPermission: Debug + PartialEq + Eq {
    fn bit_mask() -> u8;
}

macro_rules! port_permission_impl {
    ($name:ident($bit_mask:expr)) => {
        #[derive(Debug, Eq, PartialEq)]
        pub struct $name;

        impl PortPermission for $name {
            fn bit_mask() -> u8 {
                $bit_mask
            }
        }
    };
}

port_permission_impl!(PortRead(0b10));
port_permission_impl!(PortWrite(0b01));
port_permission_impl!(PortReadWrite(0b11));

#[derive(Debug, Eq, PartialEq)]
pub struct Port<S: PortSize, P: PortPermission> {
    port_address: u16,
    phantom: PhantomData<(S, P)>,
}

impl PortAllocator {
    pub const fn new() -> Self {
        Self {
            bitmap: [0; X86_PORT_BITMAP_SIZE],
        }
    }

    pub fn allocate<P: PortPermission, S: PortSize>(&mut self, address: u16) -> Option<Port<S, P>> {
        let index = (address / 4) as usize;
        let bit_offset = (address % 4) as usize * 2;
        let bit_range = bit_offset..(bit_offset + 2);
        let mut slot = self.bitmap[index];
        let port = slot.get_bits(bit_range);

        if port & P::bit_mask() == 0 {
            slot |= P::bit_mask() << bit_offset;
            self.bitmap[index] = slot;
            return Some(Port::new(address));
        }

        None
    }
}

impl<S: PortSize, P: PortPermission> Port<S, P> {
    fn new(port_address: u16) -> Self {
        Self {
            port_address,
            phantom: PhantomData,
        }
    }
}

impl<S: PortSize> Port<S, PortWrite> {
    pub unsafe fn write(&mut self, value: S::Type)
    where
        S::Type: LowerHex,
    {
        unsafe { S::write(self.port_address, value) }
    }
}

impl<S: PortSize> Port<S, PortRead> {
    pub unsafe fn read(&self) -> S::Type {
        unsafe { S::read(self.port_address) }
    }
}

impl<S: PortSize> Port<S, PortReadWrite> {
    pub unsafe fn read(&self) -> S::Type {
        unsafe { S::read(self.port_address) }
    }

    pub unsafe fn write(&mut self, value: S::Type)
    where
        S::Type: LowerHex,
    {
        unsafe { S::write(self.port_address, value) }
    }
}

select_context! {
    (Phase0, Phase1, Phase2, Phase3, FinalPhase) => {
        pub fn alloc_port<P: PortPermission, S: PortSize>(&mut self, address: u16) -> Option<Port<S, P>> {
            self.context_mut().port_allocator.allocate(address)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tester(PortAllocator);
    impl Tester {
        fn alloc_and_check<P: PortPermission, S: PortSize>(&mut self, number: u16) -> Port<S, P> {
            let port = self.0.allocate(number).expect("Failed to allocate port");
            assert_eq!(
                port,
                Port {
                    port_address: number,
                    phantom: PhantomData
                }
            );
            port
        }

        fn alloc_should_fail<P: PortPermission, S: PortSize>(&mut self, number: u16) {
            assert!(self.0.allocate::<P, S>(number).is_none());
        }
    }

    #[test_case]
    fn alloc_linear() {
        let mut test = Tester(PortAllocator::new());
        for address in 0..=u16::MAX {
            test.alloc_and_check::<PortRead, Port8Bit>(address);
        }
        for address in 0..=u16::MAX {
            test.alloc_and_check::<PortWrite, Port8Bit>(address);
        }

        for address in 0..=u16::MAX {
            test.alloc_should_fail::<PortWrite, Port8Bit>(address);
            test.alloc_should_fail::<PortRead, Port8Bit>(address);
            test.alloc_should_fail::<PortReadWrite, Port8Bit>(address);
        }

        let mut test = Tester(PortAllocator::new());

        for address in 0..=u16::MAX {
            test.alloc_and_check::<PortReadWrite, Port8Bit>(address);
        }

        for address in 0..=u16::MAX {
            test.alloc_should_fail::<PortWrite, Port8Bit>(address);
            test.alloc_should_fail::<PortRead, Port8Bit>(address);
            test.alloc_should_fail::<PortReadWrite, Port8Bit>(address);
        }
    }

    #[test_case]
    fn simple_alloc() {
        let mut test = Tester(PortAllocator::new());
        test.alloc_and_check::<PortRead, Port8Bit>(0);

        test.alloc_and_check::<PortReadWrite, Port8Bit>(u16::MAX);

        test.alloc_and_check::<PortRead, Port8Bit>(u16::MAX - 1);
        test.alloc_and_check::<PortWrite, Port8Bit>(u16::MAX - 1);

        test.alloc_should_fail::<PortWrite, Port8Bit>(u16::MAX - 1);
        test.alloc_should_fail::<PortRead, Port8Bit>(u16::MAX - 1);
        test.alloc_should_fail::<PortReadWrite, Port8Bit>(u16::MAX - 1);

        test.alloc_and_check::<PortRead, Port8Bit>(0x3FE);
        test.alloc_and_check::<PortWrite, Port8Bit>(0x3FE);
        test.alloc_and_check::<PortReadWrite, Port16Bit>(0x3FF);

        test.alloc_and_check::<PortReadWrite, Port16Bit>(0x3FD);
        test.alloc_should_fail::<PortReadWrite, Port16Bit>(0x3FD);

        test.alloc_should_fail::<PortRead, Port8Bit>(0);
        test.alloc_and_check::<PortWrite, Port8Bit>(0);
        test.alloc_should_fail::<PortReadWrite, Port8Bit>(0);

        test.alloc_and_check::<PortWrite, Port8Bit>(1);
        test.alloc_should_fail::<PortReadWrite, Port8Bit>(1);
        test.alloc_and_check::<PortRead, Port8Bit>(1);
        test.alloc_should_fail::<PortReadWrite, Port8Bit>(1);
    }
}
