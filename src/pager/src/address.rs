use core::{
    fmt,
    hash::Hash,
    ops::{Add, AddAssign, Sub},
};

use crate::PAGE_SIZE;

const PHYS_ADDR_MASK: u64 = 0x000FFFFFFFFFFFFF;

/// A structure that contains a valid physical address (bits 52-63 (inclusive) unset)
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(u64);

/// A structure that is gurentee to contain a valid ([`canonical`]) virtual address
///
/// [`canonical`]: <https://en.wikipedia.org/wiki/X86-64#Virtual_address_space_details>
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(u64);

/// A structure that contains the invalid physical address (bits 52-63 (inclusive) set)
#[repr(transparent)]
pub struct InvalidPhysAddress(pub u64);

/// A structure that contains non [`canonical`] virtual address
///
/// [`canonical`]: <https://en.wikipedia.org/wiki/X86-64#Virtual_address_space_details>
#[repr(transparent)]
pub struct NonCanonicalVirtAddress(pub u64);

/// A frame is an respresentation of an [`physical address`] that can be directly mapped in the
/// page tables, and is aligned on 4KB boundries
///
/// [`physical address`]: <https://en.wikipedia.org/wiki/X86-64#Physical_address_space_details>
#[derive(PartialEq, PartialOrd, Clone, Copy)]
pub struct Frame {
    number: u64,
}

/// A page is an respresentation of an [`virtual address`] that is aligned on 4KB boundries
///
/// [`virtual address`]: <https://en.wikipedia.org/wiki/X86-64#Virtual_address_space_details>
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Page {
    number: u64,
}

#[derive(Clone)]
pub struct PageIter {
    start: Page,
    end: Page,
}

#[derive(Clone)]
pub struct FrameIter {
    start: Frame,
    end: Frame,
}

impl Iterator for PageIter {
    type Item = Page;

    fn next(&mut self) -> Option<Page> {
        if self.start <= self.end {
            let page = self.start;
            self.start.number += 1;
            Some(page)
        } else {
            None
        }
    }
}

impl Iterator for FrameIter {
    type Item = Frame;

    fn next(&mut self) -> Option<Frame> {
        if self.start <= self.end {
            let frame = self.start;
            self.start.number += 1;
            Some(frame)
        } else {
            None
        }
    }
}

impl From<PhysAddr> for Frame {
    fn from(value: PhysAddr) -> Self {
        Self::containing_address(value)
    }
}

impl Frame {
    /// Create a frame containing the provided physical address
    ///
    /// if the address is not aligned this will create a frame contatining the frame that covers
    /// that address
    pub const fn containing_address(address: PhysAddr) -> Frame {
        Frame {
            number: address.as_u64() / PAGE_SIZE,
        }
    }

    pub const fn null() -> Frame {
        Frame { number: 0 }
    }

    /// Get the physical address back from the frame
    pub const fn start_address(&self) -> PhysAddr {
        PhysAddr::new(self.number * PAGE_SIZE)
    }

    /// Create a iterator of frame start-end (inclusive)
    pub const fn range_inclusive(start: Frame, end: Frame) -> FrameIter {
        FrameIter { start, end }
    }

    /// Get the frame number
    pub const fn number(&self) -> u64 {
        self.number
    }

    /// Add the frame number by the page number, and consume the current one
    pub const fn add_by_page(mut self, number: u64) -> Frame {
        self.number += number;
        self
    }
}

impl Page {
    /// Create a page contating the provided virtual address
    ///
    /// if the address is not aligned this will create a page contatining the frame that covers
    /// that address
    pub const fn containing_address(address: VirtAddr) -> Page {
        Page {
            number: address.0 / PAGE_SIZE,
        }
    }

    /// Just a dummy page if anyone needs it
    ///
    /// Create a deadbeef page
    pub const fn deadbeef() -> Self {
        Self { number: 0xdeadbeef }
    }

    /// Also just a dummy page if anyone needs it
    ///
    /// Create a cafebabe page
    pub const fn cafebabe() -> Self {
        Self { number: 0xcafebabe }
    }

    /// Get the start address of this frame
    ///
    /// # Note
    /// if this was created from contating address, this function does not return the original
    /// virtual address
    pub const fn start_address(&self) -> VirtAddr {
        VirtAddr::new(self.number * PAGE_SIZE)
    }

    pub const fn page_number(&self) -> u64 {
        self.number
    }

    /// Get the page 4 index of the containing page (use in [`paging`])
    ///
    /// [`paging`]: <https://wiki.osdev.org/Paging>
    pub fn p4_index(&self) -> u64 {
        (self.number >> 27) & 0o777
    }

    /// Get the page 3 index of the containing page (use in [`paging`])
    ///
    /// [`paging`]: <https://wiki.osdev.org/Paging>
    pub fn p3_index(&self) -> u64 {
        (self.number >> 18) & 0o777
    }

    /// Get the page 2 index of the containing page (use in [`paging`])
    ///
    /// [`paging`]: <https://wiki.osdev.org/Paging>
    pub fn p2_index(&self) -> u64 {
        (self.number >> 9) & 0o777
    }

    /// Get the page 1 index of the containing page (use in [`paging`])
    ///
    /// [`paging`]: <https://wiki.osdev.org/Paging>
    pub fn p1_index(&self) -> u64 {
        self.number & 0o777
    }

    /// Create a iterator of page start-end (inclusive)
    pub fn range_inclusive(start: Page, end: Page) -> PageIter {
        PageIter { start, end }
    }
}

impl From<VirtAddr> for Page {
    fn from(value: VirtAddr) -> Self {
        Self::containing_address(value)
    }
}

impl PhysAddr {
    /// Create a new physical address from u64
    ///
    /// # Panics
    ///
    /// If the bit 52-63 (inclusive) was set, this will panic
    #[inline(always)]
    pub const fn new(address: u64) -> Self {
        let truncated = Self::new_truncate(address);
        if truncated.0 != address {
            panic!("bits 52-63 in physical address was set");
        }
        truncated
    }

    /// Check if the physical address is null
    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    /// Create a new physical address from u64
    ///
    /// Returns Err if bits 52-63 (inclusive) was set
    #[inline(always)]
    pub const fn new_checked(address: u64) -> Result<Self, InvalidPhysAddress> {
        if address & PHYS_ADDR_MASK != address {
            return Err(InvalidPhysAddress(address));
        }
        // SAFETY: we already check for the bit 52-63 above
        unsafe { Ok(Self::new_unchecked(address)) }
    }

    /// Create a new physical address from u64 and truncate the bits 52-63 (inclusive)
    #[inline(always)]
    pub const fn new_truncate(address: u64) -> Self {
        // SAFETY: we truncate the 52-63 using the bits mask
        unsafe { Self::new_unchecked(address & PHYS_ADDR_MASK) }
    }

    /// Create a new physical address from u64
    ///
    /// # Safety
    ///
    /// The caller must ensure that bits 52-63 (inclusive) is set to zero
    #[inline(always)]
    pub const unsafe fn new_unchecked(address: u64) -> Self {
        Self(address)
    }

    /// Get the inner value as u64
    ///
    /// The value is gurentee to have bits 52-63 unset if this was constructed safely
    #[inline(always)]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for PhysAddr {
    type Error = InvalidPhysAddress;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new_checked(value)
    }
}

impl From<PhysAddr> for u64 {
    fn from(value: PhysAddr) -> Self {
        value.0
    }
}

impl VirtAddr {
    /// Create a new virtual address from u64
    ///
    /// # Panics
    ///
    /// If the address is not [`canonical`], this will panic
    ///
    /// [`canonical`]: <https://en.wikipedia.org/wiki/X86-64#Virtual_address_space_details>
    #[inline(always)]
    pub const fn new(address: u64) -> Self {
        if Self::is_canonical(address) {
            // SAFETY: we already check if it's a canonical or not
            unsafe { Self::new_unchecked(address) }
        } else {
            panic!("The virtual address is not caniconal and can cause gp fault");
        }
    }

    /// Create a virtual address containing the upper canonical half of 48-bit addressing
    /// (ffff_8000_0000_0000)
    pub const fn canonical_higher_half() -> Self {
        Self::new(0xffff_8000_0000_0000)
    }

    /// Create a new null virtual address
    #[inline(always)]
    pub const fn null() -> Self {
        unsafe { Self::new_unchecked(0) }
    }

    #[inline(always)]
    pub const fn max() -> Self {
        unsafe { Self::new_unchecked(0xffff_ffff_ffff_ffff) }
    }

    pub fn align_to(&self, phys: PhysAddr) -> Self {
        let misalignment = phys.as_u64() & (PAGE_SIZE - 1);
        // add it on to the virtual base
        let raw = self
            .as_u64()
            .checked_add(misalignment)
            .expect("VirtAddr overflow in align_to");
        // we know that (self + misalignment) stays canonical if self was
        unsafe { VirtAddr::new_unchecked(raw) }
    }

    /// Create a new virtual address from u64
    ///
    /// Returns Err if bits 52-63 (inclusive) was set
    #[inline(always)]
    pub const fn new_checked(address: u64) -> Result<Self, NonCanonicalVirtAddress> {
        if address & PHYS_ADDR_MASK != address {
            return Err(NonCanonicalVirtAddress(address));
        }
        // SAFETY: we already check if it's a canonical or not
        unsafe { Ok(Self::new_unchecked(address)) }
    }

    /// Create a new virtual address from u64
    ///
    /// # Safety
    ///
    /// The caller must ensure that the address is [`canonical`]
    ///
    /// [`canonical`]: <https://en.wikipedia.org/wiki/X86-64#Virtual_address_space_details>
    #[inline(always)]
    pub const unsafe fn new_unchecked(address: u64) -> Self {
        Self(address)
    }

    /// Check if the provide address is [`canonical`]
    ///
    /// [`canonical`]: <https://en.wikipedia.org/wiki/X86-64#Virtual_address_space_details>
    #[inline(always)]
    const fn is_canonical(addr: u64) -> bool {
        ((addr >> 47) == 0) || ((addr >> 47) == 0x1FFFF)
    }

    /// Get the inner value as u64
    ///
    /// The value is gurentee to be [`canonical address`] unset if this was constructed safely
    ///
    /// [`canonical address`]: <https://en.wikipedia.org/wiki/X86-64#Virtual_address_space_details>
    #[inline(always)]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Converts the address to a const raw pointer.
    #[inline(always)]
    pub const fn as_ptr<T>(self) -> *const T {
        self.as_u64() as *const T
    }

    /// Converts the address to a mutable raw pointer.
    #[inline(always)]
    pub const fn as_mut_ptr<T>(self) -> *mut T {
        self.as_ptr::<T>() as *mut T
    }

    /// Check if the virtual address is null
    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

impl Hash for VirtAddr {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // Use splitmix64
        let mut x = self.0;
        x ^= x >> 30;
        x = x.overflowing_mul(0xbf58476d1ce4e5b9).0;
        x ^= x >> 27;
        x = x.overflowing_mul(0x94d049bb133111eb).0;
        x ^= x >> 31;
        state.write_u64(x);
    }
}

impl Sub<usize> for PhysAddr {
    type Output = PhysAddr;

    fn sub(self, rhs: usize) -> Self::Output {
        Self::new(self.0 - rhs as u64)
    }
}

impl Add<u64> for PhysAddr {
    type Output = PhysAddr;

    fn add(self, rhs: u64) -> Self::Output {
        Self::new(self.0 + rhs)
    }
}

impl Add<usize> for PhysAddr {
    type Output = PhysAddr;

    fn add(self, rhs: usize) -> Self::Output {
        Self::new(self.0 + rhs as u64)
    }
}

impl AddAssign<u64> for PhysAddr {
    fn add_assign(&mut self, rhs: u64) {
        self.0 = Self::new(self.0 + rhs).0;
    }
}

impl AddAssign<usize> for PhysAddr {
    fn add_assign(&mut self, rhs: usize) {
        self.0 = Self::new(self.0 + rhs as u64).0;
    }
}

impl fmt::LowerHex for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

impl fmt::LowerHex for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

impl Sub<VirtAddr> for VirtAddr {
    type Output = VirtAddr;

    fn sub(self, rhs: VirtAddr) -> Self::Output {
        Self::new(self.0 - rhs.0)
    }
}

impl Sub<usize> for VirtAddr {
    type Output = VirtAddr;

    fn sub(self, rhs: usize) -> Self::Output {
        Self::new(self.0 - rhs as u64)
    }
}

impl Add<u64> for VirtAddr {
    type Output = VirtAddr;

    fn add(self, rhs: u64) -> Self::Output {
        Self::new(self.0 + rhs)
    }
}

impl Add<VirtAddr> for VirtAddr {
    type Output = VirtAddr;

    fn add(self, rhs: VirtAddr) -> Self::Output {
        Self::new(self.0 + rhs.0)
    }
}

impl Add<usize> for VirtAddr {
    type Output = VirtAddr;

    fn add(self, rhs: usize) -> Self::Output {
        Self::new(self.0 + rhs as u64)
    }
}

impl AddAssign<u64> for VirtAddr {
    fn add_assign(&mut self, rhs: u64) {
        self.0 = Self::new(self.0 + rhs).0;
    }
}

impl AddAssign<usize> for VirtAddr {
    fn add_assign(&mut self, rhs: usize) {
        self.0 = Self::new(self.0 + rhs as u64).0;
    }
}
