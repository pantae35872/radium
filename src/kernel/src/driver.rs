use bootbridge::BootBridge;

pub mod acpi;
pub mod display;
pub mod pci;
pub mod storage;
pub mod uefi_runtime;

pub fn init(boot_info: &BootBridge) {
    //uefi_runtime::init(boot_info);
    acpi::init(boot_info);
    storage::init();
    pci::init();
}
