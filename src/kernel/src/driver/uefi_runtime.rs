//use bootbridge::BootBridge;
//use spin::{Mutex, Once};

//pub static UEFI_RUNTIME: Once<UefiRuntime> = Once::new();
//
//pub struct UefiRuntime {
//    table: Mutex<SystemTable<Runtime>>,
//}
//
//impl UefiRuntime {
//    fn new(table: SystemTable<Runtime>) -> Self {
//        Self {
//            table: Mutex::new(table),
//        }
//    }
//
//    fn runtime_service<T>(&self, runner: impl Fn(&RuntimeServices) -> T) -> T {
//        runner(unsafe { self.table.lock().runtime_services() })
//    }
//
//    pub fn get_time(&self) -> Option<Time> {
//        self.runtime_service(|runtime_service| runtime_service.get_time().ok())
//    }
//
//    pub fn shutdown(&self) {
//        self.runtime_service(|runtime_service| {
//            runtime_service.reset(ResetType::SHUTDOWN, Status::SUCCESS, None)
//        })
//    }
//}
//
//unsafe impl Send for UefiRuntime {}
//unsafe impl Sync for UefiRuntime {}

//pub fn uefi_runtime() -> &'static UefiRuntime {
//    return UEFI_RUNTIME.get().expect("AHCI driver not initialized");
//}
//
//pub fn init(boot_info: &BootBridge) {
//log!(Trace, "Initializing uefi runtime");
//let mut runtime_table = boot_info
//    .runtime_system_table()
//    .expect("Failed to get runtime table ");
//let mut runtime_table_addr = runtime_table.get_current_system_table_addr();
//let mut runtime_map: Vec<_> = boot_info
//    .memory_map()
//    .entries()
//    .filter_map(|descriptor| match (descriptor.ty, descriptor.att) {
//        (
//            MemoryType::RUNTIME_SERVICES_CODE
//            | MemoryType::RUNTIME_SERVICES_DATA
//            | MemoryType::BOOT_SERVICES_DATA,
//            _,
//        )
//        | (_, MemoryAttribute::RUNTIME) => {
//            let size = descriptor.page_count * PAGE_SIZE;
//            let virt_addr = virt_addr_alloc(size);

//            if descriptor.phys_start < runtime_table_addr
//                && descriptor.phys_start + size > runtime_table_addr
//            {
//                runtime_table_addr = virt_addr + (runtime_table_addr - descriptor.phys_start);
//            }
//            memory_controller().lock().ident_map(
//                size,
//                descriptor.phys_start,
//                EntryFlags::PRESENT
//                    | EntryFlags::NO_CACHE
//                    | EntryFlags::WRITABLE
//                    | EntryFlags::WRITE_THROUGH,
//            );
//            let mut new_des = descriptor.clone();
//            new_des.virt_start = virt_addr;
//            Some(new_des)
//        }
//        _ => None,
//    })
//    .collect();
//runtime_map.iter().for_each(|e| {
//    let size = e.page_count * PAGE_SIZE;
//    memory_controller().lock().phy_map(
//        size,
//        e.phys_start,
//        e.virt_start,
//        EntryFlags::PRESENT
//            | EntryFlags::NO_CACHE
//            | EntryFlags::WRITABLE
//            | EntryFlags::WRITE_THROUGH,
//    );
//});
//unsafe {
//    runtime_table = runtime_table
//        .set_virtual_address_map(&mut runtime_map, runtime_table_addr)
//        .expect("Failed to initialize runtime system table");
//}
//runtime_map.iter().for_each(|e| {
//    memory_controller()
//        .lock()
//        .unmap_addr(e.phys_start, e.page_count * PAGE_SIZE);
//});
//UEFI_RUNTIME.call_once(|| UefiRuntime::new(runtime_table));
//}
