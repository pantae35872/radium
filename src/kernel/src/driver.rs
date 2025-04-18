use bootbridge::BootBridge;

pub mod acpi;
pub mod display;
pub mod pci;
pub mod pit;
pub mod storage;
pub mod uefi_runtime;

pub fn init(boot_info: &BootBridge) {
    storage::init();
    pci::init();
}
