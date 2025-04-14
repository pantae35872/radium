use crate::memory::Frame;

use super::EntryFlags;

#[derive(Debug, Clone)]
pub struct Entry(pub u64);

impl Entry {
    pub fn is_unused(&self) -> bool {
        self.0 == 0
    }

    pub fn overwriteable(&self) -> bool {
        self.flags().contains(EntryFlags::OVERWRITEABLE)
    }

    pub fn set_unused(&mut self) {
        self.0 = 0;
    }
    pub fn flags(&self) -> EntryFlags {
        EntryFlags::from_bits_truncate(self.0)
    }
    pub fn pointed_frame(&self) -> Option<Frame> {
        if self.flags().contains(EntryFlags::PRESENT) {
            Some(Frame::containing_address(self.0 & 0x000fffff_fffff000))
        } else {
            None
        }
    }
    pub fn set(&mut self, frame: Frame, flags: EntryFlags) {
        assert!(frame.start_address().as_u64() & !0x000fffff_fffff000 == 0);
        self.0 = frame.start_address().as_u64() | flags.bits();
    }
}
