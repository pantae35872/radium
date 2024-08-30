pub mod display;
pub mod pci;
pub mod storage;

pub fn init() {
    storage::init();
    pci::init();
}
