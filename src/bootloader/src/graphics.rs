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
    let framebuffer = gop.frame_buffer().as_mut_ptr() as u64;
    let framebuffer_size = gop.frame_buffer().size();

    for mode in gop.modes(system_table.boot_services()) {
        if mode.info().resolution() == (1920, 1080) {
            boot_info.init_graphics(mode.info().clone(), framebuffer, framebuffer_size);
            gop.set_mode(&mode).expect("Could not set mode");
            break;
        }
    }
}
