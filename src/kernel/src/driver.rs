use crate::MemoryController;

pub mod display;
pub mod pci;
pub mod storage;

pub fn init(memory_controller: &mut MemoryController) {
    storage::init();
    pci::init(memory_controller);
}
