use bit_field::BitField;
use conquer_once::spin::OnceCell;
use sentinel::log;
use spin::Mutex;

use crate::{
    initialization_context::{InitializationContext, Phase3},
    port::{Port, Port8Bit, PortReadWrite, PortWrite},
};

static PIT: OnceCell<Mutex<ProgrammableIntervalTimer>> = OnceCell::uninit();

struct ProgrammableIntervalTimer {
    channel0_data: Port<Port8Bit, PortReadWrite>,
    channel1_data: Port<Port8Bit, PortReadWrite>,
    channel2_data: Port<Port8Bit, PortReadWrite>,
    command: Port<Port8Bit, PortWrite>,
}

struct CommandBuilder {
    operating_mode: OperatingMode,
    access_mode: AccessMode,
    channel: Channel,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum Channel {
    Channel0 = 0b00,
    Channel1 = 0b01,
    Channel2 = 0b10,
    ReadBackCommand = 0b11,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum AccessMode {
    LatchCount = 0b00,
    LowByteOnly = 0b01,
    HiByteOnly = 0b10,
    LowHiByte = 0b11,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum OperatingMode {
    InterruptOnTerminal = 0b000,
    HardwareReTriggerableOneShot = 0b001,
    RateGenerator = 0b10,
    SquareWaveGenerator = 0b11,
    SoftwareTriggeredStrobe = 0b100,
    HardwareTriggeredStrobe = 0b101,
}

impl ProgrammableIntervalTimer {
    fn new(ctx: &mut InitializationContext<Phase3>) -> Self {
        Self {
            channel0_data: ctx.alloc_port(0x40).expect("PIC channel 0 port is taken"),
            channel1_data: ctx.alloc_port(0x41).expect("PIC channel 1 port is taken"),
            channel2_data: ctx.alloc_port(0x42).expect("PIC channel 2 port is taken"),
            command: ctx.alloc_port(0x43).expect("PIC command port is taken"),
        }
    }

    /// Default to rate generator
    fn init(&mut self) {
        self.set_freq(1000);
    }

    /// Use rate generator to generate the specify frequency
    fn set_freq(&mut self, freq: usize) {
        let cmd = CommandBuilder::new()
            .operating_mode(OperatingMode::SquareWaveGenerator)
            .access_mode(AccessMode::LowHiByte)
            .channel(Channel::Channel0)
            .build();
        log!(Debug, "PIT Cmd: {:#b}", cmd);
        unsafe { self.command.write(cmd) };
        let divsor = calculate_pit_divsor(freq);
        log!(Debug, "PIT Divsor: {}", divsor);
        unsafe { self.channel0_data.write((divsor & 0xFF) as u8) };
        unsafe { self.channel0_data.write(((divsor >> 8) & 0xFF) as u8) };
    }
}

// This magical formula is taken from https://www.reddit.com/r/osdev/comments/7gorff/pit_and_frequency/?show=original
fn calculate_pit_divsor(freq: usize) -> usize {
    const PIT_BASE_FREQUENCY: usize = 1_193_182;
    let divisor = PIT_BASE_FREQUENCY / freq;
    divisor.clamp(1, 0xFFFF)
}

impl CommandBuilder {
    fn new() -> Self {
        Self {
            operating_mode: OperatingMode::default(),
            access_mode: AccessMode::default(),
            channel: Channel::default(),
        }
    }

    fn operating_mode(&mut self, mode: OperatingMode) -> &mut Self {
        self.operating_mode = mode;
        self
    }

    fn channel(&mut self, channel: Channel) -> &mut Self {
        self.channel = channel;
        self
    }

    fn access_mode(&mut self, mode: AccessMode) -> &mut Self {
        self.access_mode = mode;
        self
    }

    fn build(&mut self) -> u8 {
        let mut result = 0;
        result.set_bit(0, false); // We didn't add support for BCD
        result.set_bits(1..4, self.operating_mode as u8);
        result.set_bits(4..6, self.access_mode as u8);
        result.set_bits(6..8, self.channel as u8);
        result
    }
}

impl Default for Channel {
    fn default() -> Self {
        Self::Channel0
    }
}

impl Default for AccessMode {
    fn default() -> Self {
        Self::LowHiByte
    }
}

impl Default for OperatingMode {
    fn default() -> Self {
        Self::RateGenerator
    }
}

pub fn init(ctx: &mut InitializationContext<Phase3>) {
    PIT.init_once(|| {
        let mut pit = ProgrammableIntervalTimer::new(ctx);
        pit.init();
        pit.into()
    });
}
