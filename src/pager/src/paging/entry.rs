use core::fmt::Debug;

use crate::address::{Frame, PhysAddr};

use super::EntryFlags;

#[derive(Clone, Copy)]
pub struct Entry(pub u64);

impl Entry {
    pub fn is_unused(&self) -> bool {
        self.0 == 0
    }

    pub fn overwriteable(&self) -> bool {
        self.flags().contains(EntryFlags::OVERWRITEABLE)
    }

    pub fn mask_flags(&self) -> u64 {
        self.0 & 0x000fffff_fffff000
    }

    pub fn set_unused(&mut self) {
        self.0 = 0;
    }
    pub fn flags(&self) -> EntryFlags {
        EntryFlags::from_bits_truncate(self.0)
    }
    pub fn pointed_frame(&self) -> Option<Frame> {
        if self.flags().contains(EntryFlags::PRESENT) {
            // SAFETY: We already mask the 52-63 (inclusive) bits
            Some(Frame::containing_address(unsafe {
                PhysAddr::new_unchecked(self.0 & 0x000fffff_fffff000)
            }))
        } else {
            None
        }
    }
    pub fn set(&mut self, frame: Frame, flags: EntryFlags) {
        assert!(frame.start_address().as_u64() & !0x000fffff_fffff000 == 0);
        self.0 = frame.start_address().as_u64() | flags.bits();
    }
}

impl Debug for Entry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{frame} : {flags:?}",
            flags = self.flags(),
            frame = self.mask_flags()
        )?;
        Ok(())
    }
}
