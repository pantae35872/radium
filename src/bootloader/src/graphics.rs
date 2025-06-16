use bootbridge::{BootBridgeBuilder, PixelBitmask, PixelFormat};
use uefi::{
    proto::console::{
        gop::{self, GraphicsOutput},
        text::Color,
    },
    table::{
        boot::{OpenProtocolAttributes, OpenProtocolParams},
        Boot, SystemTable,
    },
};

use crate::config::BootConfig;

pub fn initialize_graphics_bootloader(system_table: &mut SystemTable<Boot>) {
    if let Some(mode) = system_table.stdout().modes().max_by(|l, r| l.cmp(r)) {
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
    boot_bridge: &mut BootBridgeBuilder<impl Fn(usize) -> *mut u8>,
    config: &BootConfig,
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

    let (width, height) = config.screen_resolution();
    let mut best_mode = None;
    let mut best_score = u64::MAX;

    for mode in gop.modes(system_table.boot_services()) {
        let res = mode.info().resolution();
        let dx = res.0 as i64 - width as i64;
        let dy = res.1 as i64 - height as i64;
        let score = (dx * dx + dy * dy) as u64;

        if score < best_score {
            best_score = score;
            best_mode = Some(mode);
        }
    }

    if let Some(mode) = best_mode {
        gop.set_mode(&mode).expect("Could not set mode");
        let framebuffer = gop.frame_buffer().as_mut_ptr() as u64;
        let (horizontal, vertical) = mode.info().resolution();
        let framebuffer_len = (vertical - 1) * mode.info().stride() + (horizontal - 1) + 1;

        let gop_info = mode.info();
        boot_bridge.framebuffer_data(
            framebuffer,
            (framebuffer_len * size_of::<u32>() + 4095) & !4095,
        );
        boot_bridge.graphics_info(
            gop_info.resolution(),
            gop_info.stride(),
            match gop_info.pixel_format() {
                gop::PixelFormat::Rgb => PixelFormat::Rgb,
                gop::PixelFormat::Bgr => PixelFormat::Bgr,
                gop::PixelFormat::Bitmask => PixelFormat::Bitmask({
                    let bitmask = gop_info.pixel_bitmask().unwrap();
                    PixelBitmask {
                        red: bitmask.red,
                        green: bitmask.green,
                        blue: bitmask.blue,
                    }
                }),
                gop::PixelFormat::BltOnly => PixelFormat::BltOnly,
            },
        );
    } else {
        panic!("Could not set to the target resolution");
    }
}
