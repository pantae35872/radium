use core::slice::from_raw_parts_mut;

use common::boot::BootInformation;
use uefi::{
    proto::console::{
        gop::GraphicsOutput,
        text::{Color, OutputMode},
    },
    table::{
        boot::{OpenProtocolAttributes, OpenProtocolParams},
        Boot, SystemTable,
    },
};
use uefi_services::println;

pub fn initialize_graphics_bootloader(system_table: &mut SystemTable<Boot>) {
    let mut largest_mode: Option<OutputMode> = None;
    let mut largest_size = 0;

    for mode in system_table.stdout().modes() {
        if mode.rows() + mode.columns() > largest_size {
            largest_size = mode.rows() + mode.columns();
            largest_mode = Some(mode);
        }
    }

    if let Some(mode) = largest_mode {
        system_table
            .stdout()
            .set_mode(mode)
            .expect("Could not change text mode");
    }

    system_table
        .stdout()
        .set_color(Color::LightGreen, Color::Black)
        .expect("Failed to set color");
    system_table
        .stdout()
        .clear()
        .expect("Could not clear screen");
}

pub fn initialize_graphics_kernel(
    system_table: &mut SystemTable<Boot>,
    boot_info: &mut BootInformation,
) {
    let handle = system_table
        .boot_services()
        .get_handle_for_protocol::<GraphicsOutput>();
    let gop = unsafe {
        system_table
            .boot_services()
            .open_protocol::<GraphicsOutput>(
                OpenProtocolParams {
                    handle: handle.unwrap(),
                    agent: system_table.boot_services().image_handle(),
                    controller: None,
                },
                OpenProtocolAttributes::GetProtocol,
            )
    };
    let mut gop = gop.unwrap();
    let mut framebuffer = gop.frame_buffer();
    boot_info.framebuffer = unsafe {
        from_raw_parts_mut(
            framebuffer.as_mut_ptr() as *mut u32,
            framebuffer.size() >> size_of::<u32>(),
        )
    };

    for mode in gop.modes(system_table.boot_services()) {
        if mode.info().resolution() == (1920, 1080) {
            gop.set_mode(&mode).expect("Could not set mode");
            println!(
                "{}, {:?}, {:?}",
                mode.info().stride(),
                mode.info().pixel_format(),
                mode.info().resolution()
            );
            boot_info.gop_mode = mode;
            break;
        }
    }
}
