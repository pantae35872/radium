use core::{fmt::Debug, marker::PhantomData};

use crate::{
    PageLevel,
    address::{Frame, PageSize, PhysAddr},
    paging::table::TableLevel,
};

use super::EntryFlags;

pub struct Entry<L: TableLevel> {
    pub value: u64,
    _marker: PhantomData<L>,
}

impl<L: TableLevel> Copy for Entry<L> {}

impl<L: TableLevel> Clone for Entry<L> {
    fn clone(&self) -> Self {
        Self { value: self.value, _marker: PhantomData }
    }
}

impl<L: TableLevel> Entry<L> {
    pub fn is_unused(&self) -> bool {
        self.value == 0
    }

    pub fn mask_flags(&self) -> u64 {
        self.value & 0x000fffff_fffff000
    }

    pub fn set_unused(&mut self) {
        self.value = 0;
    }

    pub fn flags(&self) -> EntryFlags {
        EntryFlags::from_bits_truncate(self.value)
    }

    pub fn pointed_frame(&self) -> Option<Frame<L::PageSize>>
    where
        L::PageSize: PageSize,
    {
        if !self.value.is_multiple_of(L::PageSize::SIZE) {
            return None;
        }
        if !self.flags().contains(EntryFlags::PRESENT) {
            return None;
        }

        match (L::PageSize::LEVEL, self.flags().contains(EntryFlags::HUGE_PAGE)) {
            (PageLevel::Page4K, true) | (PageLevel::Page2M | PageLevel::Page1G, false) => return None,
            (PageLevel::Page1G | PageLevel::Page2M, true) | (PageLevel::Page4K, false) => {}
        };

        // SAFETY: We already mask the 52-63 (inclusive) bits
        Some(Frame::containing_address(unsafe { PhysAddr::new_unchecked(self.value & 0x000fffff_fffff000) }))
    }

    pub fn set<S: PageSize>(&mut self, frame: Frame<S>, flags: EntryFlags) {
        assert!(frame.start_address().as_u64() & !0x000fffff_fffff000 == 0);
        self.value = frame.start_address().as_u64() | flags.bits();
    }
}

impl<L: TableLevel> Debug for Entry<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{frame} : {flags:?}", flags = self.flags(), frame = self.mask_flags())?;
        Ok(())
    }
}
