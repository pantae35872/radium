use common::{boot::BootInformation, toml::parser::TomlValue};
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
    config: &TomlValue,
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

    let resolution = config
        .get("screen_resolution")
        .expect("screen_resolution not found in the config file");
    let width = resolution
        .get("width")
        .expect("width not found in the config file")
        .as_integer()
        .expect("width is not an integer") as usize;
    let height = resolution
        .get("height")
        .expect("height not found in the config file")
        .as_integer()
        .expect("height is not an integer") as usize;
    if let Some(mode) = gop
        .modes(system_table.boot_services())
        .find(|mode| mode.info().resolution() == (width, height))
    {
        gop.set_mode(&mode).expect("Could not set mode");
        let framebuffer = gop.frame_buffer().as_mut_ptr() as u64;
        let (horizontal, vertical) = mode.info().resolution();
        let framebuffer_len = (vertical - 1) * mode.info().stride() + (horizontal - 1) + 1;

        boot_info.init_graphics(mode.info().clone(), framebuffer, framebuffer_len);
    } else {
        panic!("Could not set to the target resolution");
    }
}
