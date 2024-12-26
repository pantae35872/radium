use common::boot::BootInformation;

pub mod display;
pub mod pci;
pub mod storage;
pub mod uefi_runtime;

pub fn init(boot_info: &BootInformation) {
    storage::init();
    pci::init();
    uefi_runtime::init(boot_info);
}
