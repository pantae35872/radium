use alloc::{sync::Arc, vec::Vec};
use bit_field::BitField;
use sentinel::log;
use spin::Mutex;

use crate::{
    initialization_context::{FinalPhase, InitializationContext},
    inline_if,
    port::{Port, Port32Bit, PortRead, PortReadWrite, PortWrite},
};

pub static DRIVER: Mutex<PCIControler> = Mutex::new(PCIControler::new());

const PCI_CONFIG_COMMAND_PORT: u16 = 0xCF8;
const PCI_CONFIG_DATA_PORT: u16 = 0xCFC;

#[derive(Debug, PartialEq)]
pub enum Vendor {
    Intel,
    Amd,
    Nvidia,
    Qemu,
    Unknown(u32),
}

impl Vendor {
    pub fn new(id: u32) -> Self {
        match id {
            0x8086 => Self::Intel,
            0x1022 => Self::Amd,
            0x10DE => Self::Nvidia,
            0x1234 => Self::Qemu,
            _ => Self::Unknown(id),
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            Self::Unknown(id) => *id != 0xFFFF,
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Bar {
    Memory32 {
        address: u32,
        size: u32,
        prefetchable: bool,
    },

    Memory64 {
        address: u64,
        size: u64,
        prefetchable: bool,
    },

    IO(u32),
}

#[derive(Debug, PartialEq)]
pub enum DeviceType {
    Unknown,

    // Base Class 0x00 - Devices that predate Class Codes
    LegacyVgaCompatible,
    LegacyNotVgaCompatible,

    // Base Class 0x01 - Mass Storage Controllers
    ScsiBusController,
    IdeController,
    FloppyController,
    IpiBusController,
    RaidController,
    AtaController,
    SataController,
    SasController,
    NvmeController,
    OtherMassStorageController,

    // Base Class 0x02 - Network Controllers
    EthernetController,
    TokenRingController,
    FddiController,
    AtmController,
    IsdnController,
    PicmgController,
    OtherNetworkController,

    // Base Class 0x03 - Display Controllers
    VgaCompatibleController,
    XgaController,
    ThreeDController,
    OtherDisplayController,

    // Base Class 0x04 - Multimedia Devices
    VideoDevice,
    AudioDevice,
    TelephonyDevice,
    OtherMultimediaDevice,

    // Base Class 0x05 - Memory Controllers
    RamController,
    FlashController,
    OtherMemoryController,

    // Base Class 0x06 - Bridge Devices
    HostBridge,
    IsaBridge,
    EisaBridge,
    McaBridge,
    PciPciBridge,
    PcmciaBridge,
    NuBusBridge,
    CardBusBridge,
    RacewayBridge,
    SemiTransparentPciPciBridge,
    InfinibandPciHostBridge,
    OtherBridgeDevice,

    // Base Class 0x07 - Simple Communications Controllers
    SerialController,
    ParallelPort,
    MultiportSerialController,
    Modem,
    GpibController,
    SmartCard,
    OtherCommunicationsDevice,

    // Base Class 0x08 - Generic System Peripherals
    InterruptController,
    DmaController,
    SystemTimer,
    RtcController,
    GenericPciHotPlugController,
    SdHostController,
    OtherSystemPeripheral,

    // Base Class 0x09 - Input Devices
    KeyboardController,
    Digitizer,
    MouseController,
    ScannerController,
    GameportController,
    OtherInputController,

    // Base Class 0x0a - Docking Stations
    GenericDockingStation,
    OtherDockingStation,

    // Base Class 0x0b - Processors
    Processor386,
    Processor486,
    ProcessorPentium,
    ProcessorAlpha,
    ProcessorPowerPc,
    ProcessorMips,
    CoProcessor,

    // Base Class 0x0c - Serial Bus Controllers
    FirewireController,
    AccessBusController,
    SsaBusController,
    UsbController,
    FibreChannelController,
    SmBusController,
    InfiniBandController,
    IpmiController,
    SercosController,
    CanBusController,

    // Base Class 0x0d - Wireless Controllers
    IrdaController,
    ConsumerIrController,
    RfController,
    BluetoothController,
    BroadbandController,
    Ethernet5GHzController,
    Ethernet24GHzController,
    OtherWirelessController,

    // Base Class 0x0e - Intelligent IO Controllers
    IntelligentIoController,

    // Base Class 0x0f - Satellite Communications Controllers
    TvSatelliteCommunicationsController,
    AudioSatelliteCommunicationsController,
    VoiceSatelliteCommunicationsController,
    DataSatelliteCommunicationsController,

    // Base Class 0x10 - Encryption and Decryption Controllers
    NetworkCryptionController,
    EntertainmentCryptionController,
    OtherCryptionController,

    // Base Class 0x11 - Data Acquisition and Signal Processing Controllers
    DpioModule,
    PerformanceCounter,
    CommunicationsSynchronizationController,
    ManagementCard,
    OtherSignalProcessingController,
}

impl DeviceType {
    pub fn new(base_class: u32, sub_class: u32) -> Self {
        match (base_class, sub_class) {
            (0x00, 0x00) => DeviceType::LegacyNotVgaCompatible,
            (0x00, 0x01) => DeviceType::LegacyVgaCompatible,

            (0x01, 0x00) => DeviceType::ScsiBusController,
            (0x01, 0x01) => DeviceType::IdeController,
            (0x01, 0x02) => DeviceType::FloppyController,
            (0x01, 0x03) => DeviceType::IpiBusController,
            (0x01, 0x04) => DeviceType::RaidController,
            (0x01, 0x05) => DeviceType::AtaController,
            (0x01, 0x06) => DeviceType::SataController,
            (0x01, 0x07) => DeviceType::SasController,
            (0x01, 0x08) => DeviceType::NvmeController,
            (0x01, 0x80) => DeviceType::OtherMassStorageController,

            (0x02, 0x00) => DeviceType::EthernetController,
            (0x02, 0x01) => DeviceType::TokenRingController,
            (0x02, 0x02) => DeviceType::FddiController,
            (0x02, 0x03) => DeviceType::AtmController,
            (0x02, 0x04) => DeviceType::IsdnController,
            (0x02, 0x06) => DeviceType::PicmgController,
            (0x02, 0x80) => DeviceType::OtherNetworkController,

            (0x03, 0x00) => DeviceType::VgaCompatibleController,
            (0x03, 0x01) => DeviceType::XgaController,
            (0x03, 0x02) => DeviceType::ThreeDController,
            (0x03, 0x80) => DeviceType::OtherDisplayController,

            (0x04, 0x00) => DeviceType::VideoDevice,
            (0x04, 0x01) => DeviceType::AudioDevice,
            (0x04, 0x02) => DeviceType::TelephonyDevice,
            (0x04, 0x03) => DeviceType::OtherMultimediaDevice,

            (0x05, 0x00) => DeviceType::RamController,
            (0x05, 0x01) => DeviceType::FlashController,
            (0x05, 0x02) => DeviceType::OtherMemoryController,

            (0x06, 0x00) => DeviceType::HostBridge,
            (0x06, 0x01) => DeviceType::IsaBridge,
            (0x06, 0x02) => DeviceType::EisaBridge,
            (0x06, 0x03) => DeviceType::McaBridge,
            (0x06, 0x04) => DeviceType::PciPciBridge,
            (0x06, 0x05) => DeviceType::PcmciaBridge,
            (0x06, 0x06) => DeviceType::NuBusBridge,
            (0x06, 0x07) => DeviceType::CardBusBridge,
            (0x06, 0x08) => DeviceType::RacewayBridge,
            (0x06, 0x09) => DeviceType::SemiTransparentPciPciBridge,
            (0x06, 0x0a) => DeviceType::InfinibandPciHostBridge,
            (0x06, 0x80) => DeviceType::OtherBridgeDevice,

            (0x07, 0x00) => DeviceType::SerialController,
            (0x07, 0x01) => DeviceType::ParallelPort,
            (0x07, 0x02) => DeviceType::MultiportSerialController,
            (0x07, 0x03) => DeviceType::Modem,
            (0x07, 0x04) => DeviceType::GpibController,
            (0x07, 0x05) => DeviceType::SmartCard,
            (0x07, 0x80) => DeviceType::OtherCommunicationsDevice,

            (0x08, 0x00) => DeviceType::InterruptController,
            (0x08, 0x01) => DeviceType::DmaController,
            (0x08, 0x02) => DeviceType::SystemTimer,
            (0x08, 0x03) => DeviceType::RtcController,
            (0x08, 0x04) => DeviceType::GenericPciHotPlugController,
            (0x08, 0x05) => DeviceType::SdHostController,
            (0x08, 0x80) => DeviceType::OtherSystemPeripheral,

            (0x09, 0x00) => DeviceType::KeyboardController,
            (0x09, 0x01) => DeviceType::Digitizer,
            (0x09, 0x02) => DeviceType::MouseController,
            (0x09, 0x03) => DeviceType::ScannerController,
            (0x09, 0x04) => DeviceType::GameportController,
            (0x09, 0x80) => DeviceType::OtherInputController,

            (0x0a, 0x00) => DeviceType::GenericDockingStation,
            (0x0a, 0x80) => DeviceType::OtherDockingStation,

            (0x0b, 0x00) => DeviceType::Processor386,
            (0x0b, 0x01) => DeviceType::Processor486,
            (0x0b, 0x02) => DeviceType::ProcessorPentium,
            (0x0b, 0x10) => DeviceType::ProcessorAlpha,
            (0x0b, 0x20) => DeviceType::ProcessorPowerPc,
            (0x0b, 0x30) => DeviceType::ProcessorMips,
            (0x0b, 0x40) => DeviceType::CoProcessor,

            (0x0c, 0x00) => DeviceType::FirewireController,
            (0x0c, 0x01) => DeviceType::AccessBusController,
            (0x0c, 0x02) => DeviceType::SsaBusController,
            (0x0c, 0x03) => DeviceType::UsbController,
            (0x0c, 0x04) => DeviceType::FibreChannelController,
            (0x0c, 0x05) => DeviceType::SmBusController,
            (0x0c, 0x06) => DeviceType::InfiniBandController,
            (0x0c, 0x07) => DeviceType::IpmiController,
            (0x0c, 0x08) => DeviceType::SercosController,
            (0x0c, 0x09) => DeviceType::CanBusController,

            (0x0d, 0x00) => DeviceType::IrdaController,
            (0x0d, 0x01) => DeviceType::ConsumerIrController,
            (0x0d, 0x10) => DeviceType::RfController,
            (0x0d, 0x11) => DeviceType::BluetoothController,
            (0x0d, 0x12) => DeviceType::BroadbandController,
            (0x0d, 0x20) => DeviceType::Ethernet5GHzController,
            (0x0d, 0x21) => DeviceType::Ethernet24GHzController,
            (0x0d, 0x80) => DeviceType::OtherWirelessController,

            (0x0e, 0x00) => DeviceType::IntelligentIoController,

            (0x0f, 0x00) => DeviceType::TvSatelliteCommunicationsController,
            (0x0f, 0x01) => DeviceType::AudioSatelliteCommunicationsController,
            (0x0f, 0x02) => DeviceType::VoiceSatelliteCommunicationsController,
            (0x0f, 0x03) => DeviceType::DataSatelliteCommunicationsController,

            (0x10, 0x00) => DeviceType::NetworkCryptionController,
            (0x10, 0x10) => DeviceType::EntertainmentCryptionController,
            (0x10, 0x80) => DeviceType::OtherCryptionController,

            (0x11, 0x00) => DeviceType::DpioModule,
            (0x11, 0x01) => DeviceType::PerformanceCounter,
            (0x11, 0x10) => DeviceType::CommunicationsSynchronizationController,
            (0x11, 0x20) => DeviceType::ManagementCard,
            (0x11, 0x80) => DeviceType::OtherSignalProcessingController,

            _ => DeviceType::Unknown,
        }
    }
}

pub struct PciHeader<'a> {
    bus: u8,
    device: u8,
    function: u8,
    command_port: &'a mut Port<Port32Bit, PortWrite>,
    data_port: &'a mut Port<Port32Bit, PortReadWrite>,
}

impl<'a> PciHeader<'a> {
    pub fn new(
        command_port: &'a mut Port<Port32Bit, PortWrite>,
        data_port: &'a mut Port<Port32Bit, PortReadWrite>,
        bus: u8,
        device: u8,
        function: u8,
    ) -> Self {
        Self {
            bus,
            device,
            function,
            command_port,
            data_port,
        }
    }

    pub fn bus(&self) -> u8 {
        return self.bus;
    }

    pub fn device(&self) -> u8 {
        return self.device;
    }

    pub fn function(&self) -> u8 {
        return self.function;
    }

    pub unsafe fn read<T>(&mut self, offset: u32) -> u32 {
        unsafe {
            let bus = self.bus() as u32;
            let device = self.device() as u32;
            let func = self.function() as u32;
            let address = (bus << 16) | (device << 11) | (func << 8) | (offset & 0xFC) | 0x80000000;

            self.command_port.write(address);

            let offset = (offset & 0b11) * 8;
            let value = self.data_port.read();

            match core::mem::size_of::<T>() {
                1 => (value >> offset) as u8 as u32,
                2 => (value >> offset) as u16 as u32,
                4 => value,
                width => unreachable!("unknown PCI read width: {}", width),
            }
        }
    }

    unsafe fn write<T>(&mut self, offset: u32, value: u32) {
        unsafe {
            let current = self.read::<u32>(offset);

            let bus = self.bus() as u32;
            let device = self.device() as u32;
            let func = self.function() as u32;

            let address = (bus << 16) | (device << 11) | (func << 8) | (offset & 0xFC) | 0x80000000;
            let noffset = (offset & 0b11) * 8;

            self.command_port.write(address);
            match core::mem::size_of::<T>() {
                1 => {
                    let mask = !(0xffu32 << offset);
                    let value = (current & mask) | ((value & 0xff) << offset);
                    self.data_port.write(value);
                }
                2 => {
                    let mask = !(0xffffu32 << noffset);
                    let value = (current & mask) | ((value & 0xffff) << noffset);
                    self.data_port.write(value);
                }
                4 => self.data_port.write(value), // u32
                width => unreachable!("unknown PCI write width: {}", width),
            }
        }
    }

    pub fn enable_mmio(&mut self) {
        let command = unsafe { self.read::<u16>(0x04) };

        unsafe { self.write::<u16>(0x04, command | (1 << 1)) };
    }

    pub fn enable_bus_mastering(&mut self) {
        let command = unsafe { self.read::<u16>(0x04) };
        unsafe { self.write::<u16>(0x04, command | (1 << 2)) };
    }

    pub fn disable_legacy_irq(&mut self) {
        let command = unsafe { self.read::<u16>(0x04) };
        unsafe { self.write::<u16>(0x04, command | (1 << 10)) };
    }

    pub fn get_device(&mut self) -> DeviceType {
        let id = unsafe { self.read::<u32>(0x08) };

        return DeviceType::new(id.get_bits(24..32), id.get_bits(16..24));
    }

    pub fn get_vendor(&mut self) -> Vendor {
        return unsafe { Vendor::new(self.read::<u16>(0x00)) };
    }

    pub fn has_multiple_functions(&mut self) -> bool {
        unsafe { self.read::<u32>(0x0c) }.get_bit(23)
    }

    pub fn get_bar(&mut self, bar: u8) -> Option<Bar> {
        if bar > 5 {
            return None;
        }

        let offset = 0x10 + (bar as u16) * 4;
        let bar = unsafe { self.read::<u32>(offset.into()) };

        if bar.get_bit(0) {
            return Some(Bar::IO(bar.get_bits(2..32)));
        } else {
            let prefetchable = bar.get_bit(3);
            let address = bar.get_bits(4..32) << 4;

            let size = unsafe {
                self.write::<u32>(offset.into(), 0xffffffff);
                let mut readback = self.read::<u32>(offset.into());
                self.write::<u32>(offset.into(), address);

                if readback == 0x0 {
                    return None;
                }

                readback.set_bits(0..4, 0);
                1 << readback.trailing_zeros()
            };

            match bar.get_bits(1..3) {
                0b00 => Some(Bar::Memory32 {
                    address,
                    size,
                    prefetchable,
                }),

                0b10 => {
                    let address = {
                        let mut address = address as u64;

                        address.set_bits(
                            32..64,
                            unsafe { self.read::<u32>((offset + 4).into()) }.into(),
                        );

                        address
                    };

                    Some(Bar::Memory64 {
                        address,
                        size: size as u64,
                        prefetchable,
                    })
                }

                _ => None,
            }
        }
    }
}

pub trait PciDeviceHandle: Sync + Send {
    fn handles(&self, vendor_id: Vendor, device_id: DeviceType) -> bool;

    fn start(&self, header: &PciHeader);
}

struct PciDevice {
    handle: Arc<dyn PciDeviceHandle>,
}

pub struct PCIControler {
    drivers: Vec<PciDevice>,
}

impl PCIControler {
    const fn new() -> Self {
        Self {
            drivers: Vec::new(),
        }
    }
}

pub fn register_driver(handle: Arc<dyn PciDeviceHandle>) {
    DRIVER.lock().drivers.push(PciDevice { handle });
}

pub fn init(ctx: &mut InitializationContext<FinalPhase>) {
    let mut pci_config_data_port = ctx
        .alloc_port(PCI_CONFIG_DATA_PORT)
        .expect("PCI Data port was taken");
    let mut pci_config_command_port = ctx
        .alloc_port(PCI_CONFIG_COMMAND_PORT)
        .expect("PCI Command port was taken");
    for bus in 0..255 {
        for device in 0..32 {
            let function_count = inline_if!(
                PciHeader::new(
                    &mut pci_config_command_port,
                    &mut pci_config_data_port,
                    bus,
                    device,
                    0
                )
                .has_multiple_functions(),
                8,
                1
            );
            for function in 0..function_count {
                let mut device = PciHeader::new(
                    &mut pci_config_command_port,
                    &mut pci_config_data_port,
                    bus,
                    device,
                    function,
                );

                if !device.get_vendor().is_valid() {
                    continue;
                }

                log!(
                    Info,
                    "Found PCI Device. Vendor: {:?}, Device: {:?}",
                    device.get_vendor(),
                    device.get_device()
                );

                for driver in &DRIVER.lock().drivers {
                    if driver
                        .handle
                        .handles(device.get_vendor(), device.get_device())
                    {
                        driver.handle.start(&device);
                    }
                }
            }
        }
    }
}
