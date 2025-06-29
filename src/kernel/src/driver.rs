use pager::address::VirtAddr;
use santa::SymbolResolver;
use sentinel::{LoggerBackend, get_logger};

use crate::initialization_context::{End, InitializationContext};

pub mod acpi;
pub mod display;
pub mod pci;
pub mod pit;
pub mod uefi_runtime;

pub fn init(ctx: &mut InitializationContext<End>) {
    pci::init(ctx);
}

pub struct DriverReslover;

unsafe impl SymbolResolver for DriverReslover {
    fn resolve(&self, symbol: &str) -> Option<VirtAddr> {
        fn get_klogger() -> &'static dyn LoggerBackend {
            get_logger().expect("Logger is not inialized")
        }
        match symbol {
            "kpanic" => Some(crate::panic as usize),
            "get_klogger" => Some(get_klogger as usize),
            _ => None,
        }
        .map(|a| VirtAddr::new(a as u64))
    }
}

//pub fn load() {
//    let packed = cpu_local()
//        .ctx()
//        .lock()
//        .context_mut()
//        .boot_bridge
//        .packed_drivers();
//    for driver in packed.iter() {
//        let driver_elf = Elf::new(driver.data).expect("Driver elf not valid");
//        let start = cpu_local().ctx().lock().map(
//            driver_elf.max_memory_needed(),
//            EntryFlags::WRITABLE | EntryFlags::NEEDS_REMAP,
//        );
//
//        // SAFETY: This is valid as we've map the pages above with max_memory_needed
//        unsafe { driver_elf.load_data(start.start_address().as_mut_ptr()) };
//        driver_elf
//            .apply_relocations(start.start_address(), &DriverReslover)
//            .expect("Failed to apply relocation to driver");
//
//        cpu_local().ctx().lock().virtually_map(
//            &driver_elf,
//            start.start_address(),
//            phys_start.start_address(),
//        );
//
//        let init_fn = driver_elf
//            .lookup_symbol("init", start.start_address())
//            .expect("init fn not found in driver");
//        let init_fn: extern "C" fn() = unsafe { core::mem::transmute(init_fn.as_u64()) };
//        init_fn();
//    }
//}
