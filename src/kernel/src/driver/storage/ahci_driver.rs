use core::alloc::Layout;
use core::error::Error;
use core::fmt::Display;
use core::future::Future;
use core::intrinsics::size_of;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::{self, write_bytes};
use core::task::Poll;
use core::{u32, usize};

use alloc::alloc::alloc;
use alloc::sync::Arc;
use bit_field::BitField;
use lazy_static::lazy_static;
use pager::address::{PhysAddr, VirtAddr};
use spin::mutex::Mutex;
use spin::Once;

use crate::driver::pci::{self, register_driver, Bar, DeviceType, PciDeviceHandle, Vendor};
use crate::log;
use crate::memory::paging::EntryFlags;
use crate::memory::{memory_controller, virt_addr_alloc};
use crate::utils::VolatileCell;

use super::{DmaBuffer, DmaRequest, Drive, DriveCommand};

pub static DRIVER: Once<Arc<AhciDriver>> = Once::new();

lazy_static! {
    pub static ref ABAR_START: u64 = virt_addr_alloc(0xFF);
}
pub const ABAR_SIZE: u64 = size_of::<HbaMem>() as u64;

#[derive(Debug)]
pub enum SataDriveError {
    NoCmdSlot,
    DmaRequest,
    TaskFileError(HbaPortSerr),
    DriveNotFound(usize),
}

impl Display for SataDriveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoCmdSlot => write!(f, "Cannot find free command list entry"),
            Self::TaskFileError(serr) => {
                write!(f, "Execute command with task file error: {:?}", serr)
            }
            Self::DriveNotFound(id) => write!(f, "Trying to get drive with id: {}", id),
            Self::DmaRequest => write!(f, "Failed to request dma memory"),
        }
    }
}

impl Error for SataDriveError {}

#[derive(PartialEq)]
enum HbaPortDd {
    None = 0,
    PresentNotE = 1,
    PresentAndE = 3,
    Offline = 4,
}

enum HbaPortIpm {
    None = 0,
    Active = 1,
    Partial = 2,
    Slumber = 6,
    DevSleep = 8,
}

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
struct HbaCmdHeaderFlags(u16);

bitflags! {
    impl HbaCmdHeaderFlags: u16 {
        const A = 1 << 5; // ATAPI
        const W = 1 << 6; // Write
        const P = 1 << 7; // Prefetchable
        const R = 1 << 8; // Reset
        const B = 1 << 9; // Bist
        const C = 1 << 10; // Clear Busy upon R_OK
    }
}

impl HbaCmdHeaderFlags {
    #[inline]
    fn set_cfl(&mut self, size: usize) {
        self.0.set_bits(0..=4, size as _);
    }
}

#[repr(C)]
pub struct HbaCmdHeader {
    // DWORD 0
    flags: VolatileCell<HbaCmdHeaderFlags>,
    prdtl: VolatileCell<u16>,
    // DWORD 1
    prdbc: VolatileCell<u32>,
    // DWORD 2-3
    ctba: VolatileCell<PhysAddr>,
    // DWORD 4-7
    rsv1: [VolatileCell<u32>; 4],
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
enum FisType {
    RegH2D = 0x27,
}

#[derive(Debug)]
#[repr(C)]
pub struct HbaMem {
    // 0x00 - 0x2B, Generic Host Control
    cap: VolatileCell<HbaCapabilities>, // 0x00, Host capability
    ghc: VolatileCell<u32>,             // 0x04, Global host control
    is: VolatileCell<u32>,              // 0x08, Interrupt status
    pi: VolatileCell<u32>,              // 0x0C, Port implemented
    vs: VolatileCell<u32>,              // 0x10, Version
    ccc_ctl: VolatileCell<u32>,         // 0x14, Command completion coalescing control
    ccc_pts: VolatileCell<u32>,         // 0x18, Command completion coalescing ports
    em_loc: VolatileCell<u32>,          // 0x1C, Enclosure management location
    em_ctl: VolatileCell<u32>,          // 0x20, Enclosure management control
    cap2: VolatileCell<u32>,            // 0x24, Host capabilities extended
    bohc: VolatileCell<u32>,            // 0x28, BIOS/OS handoff control and status

    // 0x2C - 0x9F, Reserved
    rsv: [VolatileCell<u8>; 0xA0 - 0x2C],

    // 0xA0 - 0xFF, Vendor specific registers
    vendor: [VolatileCell<u8>; 0x100 - 0xA0],
}

#[derive(Debug)]
#[repr(C)]
pub struct HbaPRDTEntry {
    dba: VolatileCell<PhysAddr>,
    _reserved: VolatileCell<u32>, // Reserved

    // DW3
    dw3: VolatileCell<u32>,
}

impl HbaPRDTEntry {
    pub fn set_i(&mut self, value: bool) {
        let mut old = self.dw3.get();
        old.set_bit(31, value);
        self.dw3.set(old);
    }

    pub fn set_dbc(&mut self, value: u32) {
        let mut old = self.dw3.get();
        old.set_bits(0..21, value);

        self.dw3.set(old);
    }
}

#[derive(Debug)]
#[repr(C)]
struct HbaCmdTbl {
    // 0x00
    cfis: [u8; 64], // Command FIS

    // 0x40
    acmd: [VolatileCell<u8>; 16], // ATAPI command, 12 or 16 bytes

    // 0x50
    _reserved: [VolatileCell<u8>; 48], // Reserved

    // 0x80
    prdt_entry: [HbaPRDTEntry; 8], // Physical region descriptor table entries, 0 ~ 65535
}

#[derive(Debug)]
#[repr(C)]
pub struct FisRegH2D {
    command: VolatileCell<u8>,  // Command register
    featurel: VolatileCell<u8>, // Feature register, 7:0

    // DWORD 1
    lba0: VolatileCell<u8>,   // LBA low register, 7:0
    lba1: VolatileCell<u8>,   // LBA mid register, 15:8
    lba2: VolatileCell<u8>,   // LBA high register, 23:16
    device: VolatileCell<u8>, // Device register

    // DWORD 2
    lba3: VolatileCell<u8>,     // LBA register, 31:24
    lba4: VolatileCell<u8>,     // LBA register, 39:32
    lba5: VolatileCell<u8>,     // LBA register, 47:40
    featureh: VolatileCell<u8>, // Feature register, 15:8

    // DWORD 3
    count: VolatileCell<u16>,
    icc: VolatileCell<u8>,     // Isochronous command completion
    control: VolatileCell<u8>, // Control register

    // DWORD 4
    rsv1: [VolatileCell<u8>; 4], // Reserved
}

impl FisRegH2D {
    fn set_lba(&mut self, lba: u64) {
        self.lba0.set(lba.get_bits(0..8) as u8);
        self.lba1.set(lba.get_bits(8..16) as u8);
        self.lba2.set(lba.get_bits(16..24) as u8);
        self.lba3.set(lba.get_bits(24..32) as u8);
        self.lba4.set(lba.get_bits(32..40) as u8);
        self.lba5.set(lba.get_bits(40..48) as u8);
    }
}

impl FisRegister for FisRegH2D {
    fn fis_type() -> FisType {
        FisType::RegH2D
    }
}

impl FisRegFlags<FisRegH2D> {
    fn set_command(&mut self, value: bool) {
        let mut new = self.flags.get();
        new.set_bit(7, value);
        self.flags.set(new);
    }
}

#[repr(C)]
struct FisRegInner<T: FisRegister> {
    fis_type: VolatileCell<FisType>,
    flags: FisRegFlags<T>,
    fis: T,
}

#[repr(C)]
struct FisRegFlags<T: FisRegister> {
    flags: VolatileCell<u8>,
    phantom: PhantomData<T>,
}

struct FisReg<'a, T: FisRegister> {
    inner: &'a mut FisRegInner<T>,
}

impl<T: FisRegister> FisRegFlags<T> {
    #[allow(dead_code)]
    fn set_port_multiplier(&mut self, value: u8) {
        let mut new = self.flags.get();
        new.set_bits(0..4, value);
        self.flags.set(new);
    }

    #[allow(dead_code)]
    fn port_multiplier(&mut self) -> u8 {
        self.flags.get().get_bits(0..4)
    }
}

impl<'a, T: FisRegister> FisReg<'a, T> {
    unsafe fn new(buffer: &'a mut [u8; 64]) -> Self {
        assert!(size_of::<FisRegInner<T>>() <= 64); // Fis register must not be larger than the
                                                    // buffer
        buffer.fill(0);
        let inner: &mut FisRegInner<T> = core::mem::transmute(buffer);
        inner.fis_type.set(T::fis_type());
        Self { inner }
    }

    fn flags(&mut self) -> &mut FisRegFlags<T> {
        &mut self.inner.flags
    }
}

impl<'a, T: FisRegister> Deref for FisReg<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner.fis
    }
}

impl<'a, T: FisRegister> DerefMut for FisReg<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.fis
    }
}

trait FisRegister {
    fn fis_type() -> FisType;
}

impl HbaCmdTbl {
    pub fn get_prdt_entry(&mut self, index: usize) -> &mut HbaPRDTEntry {
        &mut self.prdt_entry[index]
    }

    fn command_fis<T: FisRegister>(&mut self) -> FisReg<T> {
        let fis_reg = unsafe { FisReg::new(&mut self.cfis) };
        return fis_reg;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct HbaCapabilities: u32 {
        const S64A = 1 << 31;
        const SNCQ = 1 << 30;
        const SSNTF = 1 << 29;
        const SMPS = 1 << 28;
        const SSS = 1 << 27;
        const SALP = 1 << 26;
        const SAL = 1 << 25;
        const SCLO = 1 << 24;
        const SPM = 1 << 17;
        const FBSS = 1 << 16;
        const PMD = 1 << 15;
        const SSC = 1 << 14;
        const PSC = 1 << 13;
        const CCCS = 1 << 7;
        const EMS = 1 << 6;
        const SXS = 1 << 5;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct HbaPortCmd: u32 {
        const ST = 1 << 0; // Start
        const SUD = 1 << 1; // Spin-Up Device
        const POD = 1 << 2; // Power On Device
        const CLO = 1 << 3; // Command List Override
        const FRE = 1 << 4; // FIS Receive Enable
        const MPSS = 1 << 13; // Mechanical Presence Switch State
        const FR = 1 << 14; // FIS Receive Running
        const CR = 1 << 15; // Command List Running
        const CPS = 1 << 16; // Cold Presence State
        const PMA = 1 << 17; // Port Multiplier Attached
        const HPCP = 1 << 18; // Hot Plug Capable Port
        const MSPC = 1 << 19; // Mechanical Presence Switch Attached to Port
        const CPD = 1 << 20; // Cold Presence Detection
        const ESP = 1 << 21; // External SATA Port
        const FBSCP = 1 << 22; // FIS-based Switching Capable Port
        const APSTE = 1 << 23; // Automatic Partial to Slumber Transition Enabled
        const ATAPI = 1 << 24; // Device is ATAPI
        const DLAE = 1 << 25; // Drive LED on ATAPI Enable
        const ALPE = 1 << 26; // Aggressive Link Power Management Enable
        const ASP = 1 << 27; // Aggressive Slumber / Partial
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct HbaPortSerr: u32 {
        const Exchanged = 1 << 26;
        const UnknownFisType = 1 << 25;
        const TransportStateTransision = 1 << 24;
        const LinkSequenceError = 1 << 23;
        const HandshakeError = 1 << 22;
        const CrcError = 1 << 21;
        const DisparityError = 1 << 20;
        const DecodeError = 1 << 19;
        const CommWake = 1 << 18;
        const PhyInternalError = 1 << 17;
        const PhyRdyChange = 1 << 16;
        const InternalError = 1 << 11;
        const ProtocolError = 1 << 10;
        const PersistentCommunicationOrDataIntegrityError = 1 << 9;
        const TransientDataIntegrityError = 1 << 8;
        const RecoveredCommunicationsError = 1 << 1;
        const RecoveredDataIntegrityError = 1 << 0;
    }
}

bitflags! {
    #[derive(Debug, Copy, Clone)]
    struct HbaPortIS: u32 {
        const DHRS = 1 << 0; // Device to Host Register FIS Interrupt
        const PSS = 1 << 1; // PIO Setup FIS Interrupt
        const DSS = 1 << 2; // DMA Setup FIS Interrupt
        const SDBS = 1 << 3; // Set Device Bits Interrupt
        const UFS = 1 << 4; // Unknown FIS Interrupt
        const DPS = 1 << 5; // Descriptor Processed
        const PCS = 1 << 6; // Port Connect Change Status
        const DMPS = 1 << 7; // Device Mechanical Presence Status
        const PRCS = 1 << 22; // PhyRdy Change Status
        const IPMS = 1 << 23; // Incorrect Port Multiplier Status
        const OFS = 1 << 24; // Overflow Status
        const INFS = 1 << 26; // Interface Not-fatal Error Status
        const IFS = 1 << 27; // Interface Fatal Error Status
        const HBDS = 1 << 28; // Host Bus Data Error Status
        const HBFS = 1 << 29; // Host Bus Fatal Error Status
        const TFES = 1 << 30; // Task File Error Status
        const CPDS = 1 << 31; // Cold Port Detect Status
    }
}

bitflags! {
    #[derive(Debug, Copy, Clone)]
    struct HbaPortIE: u32 {
        const DHRE = 1 << 0; // Device to Host Register FIS Interrupt
        const PSE = 1 << 1; // PIO Setup FIS Interrupt
        const DSE = 1 << 2; // DMA Setup FIS Interrupt
        const SDBE = 1 << 3; // Set Device Bits Interrupt
        const UFE = 1 << 4; // Unknown FIS Interrupt
        const DPE = 1 << 5; // Descriptor Processed
        const PCE = 1 << 6; // Port Connect Change Status
        const DMPE = 1 << 7; // Device Mechanical Presence Status
        const PRCE = 1 << 22; // PhyRdy Change Status
        const IPME = 1 << 23; // Incorrect Port Multiplier Status
        const OFE= 1 << 24; // Overflow Status
        const INFE = 1 << 26; // Interface Not-fatal Error Status
        const IFE = 1 << 27; // Interface Fatal Error Status
        const HBDE = 1 << 28; // Host Bus Data Error Status
        const HBFE = 1 << 29; // Host Bus Fatal Error Status
        const TFEE = 1 << 30; // Task File Error Status
        const CPDE = 1 << 31; // Cold Port Detect Status
    }
}

impl HbaCapabilities {
    #[allow(dead_code)]
    fn number_of_ports(&self) -> u8 {
        self.bits().get_bits(0..4) as u8 + 1
    }

    fn number_of_slots(&self) -> u8 {
        self.bits().get_bits(8..12) as u8 + 1
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
struct HbaSataStatus(u32);

impl HbaSataStatus {
    fn device_detection(&self) -> HbaPortDd {
        match self.0.get_bits(0..=3) {
            0 => HbaPortDd::None,
            1 => HbaPortDd::PresentNotE,
            3 => HbaPortDd::PresentAndE,
            4 => HbaPortDd::Offline,
            v => panic!("Invalid HbaPortSstsRegDet {}", v),
        }
    }

    fn interface_power_management(&self) -> HbaPortIpm {
        match self.0.get_bits(8..=11) {
            0 => HbaPortIpm::None,
            1 => HbaPortIpm::Active,
            2 => HbaPortIpm::Partial,
            6 => HbaPortIpm::Slumber,
            8 => HbaPortIpm::DevSleep,
            v => panic!("Invalid HbaPortSstsRegIpm {}", v),
        }
    }
}

#[derive(Debug)]
enum AhciDriveType {
    Sata,
    SataPI,
    Semb,
    Pm,
}

impl AhciDriveType {
    fn from_signature(sig: u32) -> Self {
        return match sig {
            0xEB140101 => Self::SataPI,
            0xC33C0101 => Self::Semb,
            0x96690101 => Self::Pm,
            _ => Self::Sata,
        };
    }
}

impl Display for AhciDriveType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Sata => write!(f, "Sata drive"),
            Self::Semb => write!(f, "Semb drive"),
            Self::Pm => write!(f, "Pm drive"),
            Self::SataPI => write!(f, "SataPI drive"),
        }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct HbaPort {
    clb: VolatileCell<PhysAddr>,
    fb: VolatileCell<PhysAddr>,
    is: VolatileCell<HbaPortIS>,         // 0x10, interrupt status
    ie: VolatileCell<HbaPortIE>,         // 0x14, interrupt enable
    cmd: VolatileCell<HbaPortCmd>,       // 0x18, command and status
    _reserved: VolatileCell<u32>,        // 0x1C, Reserved
    tfd: VolatileCell<u32>,              // 0x20, task file data
    sig: VolatileCell<u32>,              // 0x24, signature
    ssts: VolatileCell<HbaSataStatus>,   // 0x28, SATA status (SCR0:SStatus)
    sctl: VolatileCell<u32>,             // 0x2C, SATA control (SCR2:SControl)
    serr: VolatileCell<HbaPortSerr>,     // 0x30, SATA error (SCR1:SError)
    sact: VolatileCell<u32>,             // 0x34, SATA active (SCR3:SActive)
    ci: VolatileCell<u32>,               // 0x38, command issue
    sntf: VolatileCell<u32>,             // 0x3C, SATA notification (SCR4:SNotification)
    fbs: VolatileCell<u32>,              // 0x40, FIS-based switch control
    _reserved1: [VolatileCell<u32>; 11], // 0x44 ~ 0x6F, Reserved
    vendor: [VolatileCell<u32>; 4],      // 0x70 ~ 0x7F, vendor specific
}

pub struct SataPort {
    hba_port: &'static mut HbaPort,
    clb: VirtAddr,
    fb: VirtAddr,
    ctba: [VirtAddr; 32],
    cap: HbaCapabilities,
}

impl SataPort {
    fn cmd_header(&mut self, slot: usize) -> &mut HbaCmdHeader {
        unsafe { &mut *(self.clb.as_mut_ptr::<HbaCmdHeader>().add(slot)) }
    }

    fn cmd_tbl(&mut self, slot: usize) -> &mut HbaCmdTbl {
        unsafe { &mut *(self.ctba[slot].as_mut_ptr::<HbaCmdTbl>()) }
    }

    fn start_cmd(&mut self) {
        while self.hba_port.cmd.get().contains(HbaPortCmd::CR) {
            core::hint::spin_loop();
        }
        let value = self.hba_port.cmd.get() | HbaPortCmd::FRE | HbaPortCmd::ST;
        self.hba_port.cmd.set(value);
    }

    fn stop_cmd(&mut self) {
        let mut cmd = self.hba_port.cmd.get();
        cmd.remove(HbaPortCmd::FRE | HbaPortCmd::ST);

        self.hba_port.cmd.set(cmd);

        while self
            .hba_port
            .cmd
            .get()
            .intersects(HbaPortCmd::FR | HbaPortCmd::CR)
        {
            core::hint::spin_loop();
        }
    }

    fn check_type(&mut self) -> Option<AhciDriveType> {
        let status = self.hba_port.ssts.get();

        let ipm = status.interface_power_management();
        let dd = status.device_detection();

        if let (HbaPortDd::PresentAndE, HbaPortIpm::Active) = (dd, ipm) {
            return Some(AhciDriveType::from_signature(self.hba_port.sig.get()));
        } else {
            return None;
        }
    }

    fn rebase(&mut self) {
        self.stop_cmd();
        let virt_clb: VirtAddr = unsafe {
            VirtAddr::new(alloc(
                Layout::from_size_align(size_of::<HbaCmdHeader>() * 32, 1024).unwrap(),
            ) as u64)
        };

        unsafe {
            write_bytes(
                virt_clb.as_mut_ptr::<u8>(),
                0,
                size_of::<HbaCmdHeader>() * 32,
            );
        }
        self.hba_port
            .clb
            .set(memory_controller().lock().get_physical(virt_clb).unwrap());
        let virt_fb: VirtAddr =
            unsafe { VirtAddr::new(alloc(Layout::from_size_align(0xFF, 256).unwrap()) as u64) };
        unsafe {
            write_bytes(virt_fb.as_mut_ptr::<u8>(), 0, 0xFF);
        }
        self.hba_port
            .fb
            .set(memory_controller().lock().get_physical(virt_fb).unwrap());
        self.fb = virt_fb;
        self.clb = virt_clb;

        let cmdheader = unsafe { &mut *(virt_clb.as_mut_ptr() as *mut [HbaCmdHeader; 32]) };
        for i in 0..32 {
            cmdheader[i].prdtl.set(8);
            let virt_ctba: VirtAddr =
                unsafe { VirtAddr::new(alloc(Layout::from_size_align(4096, 128).unwrap()) as u64) };
            unsafe {
                ptr::write_bytes(virt_ctba.as_mut_ptr::<u8>(), 0, 4096);
            }

            self.ctba[i] = virt_ctba;
            cmdheader[i]
                .ctba
                .set(memory_controller().lock().get_physical(virt_ctba).unwrap());
        }
        // Reset port for good mesure
        self.hba_port
            .sctl
            .set(*self.hba_port.sctl.get().set_bits(0..3, 1));
        // TODO: use a proper timer for this
        for _ in 0..1000000 {}
        self.hba_port
            .sctl
            .set(*self.hba_port.sctl.get().set_bits(0..3, 0));
        // Wait for reestablished
        while self.hba_port.ssts.get().device_detection() != HbaPortDd::PresentAndE {
            core::hint::spin_loop();
        }
        self.hba_port.serr.set(HbaPortSerr::all());
        self.hba_port.ie.set(HbaPortIE::all());
        let mut cmd = self.hba_port.cmd.get();
        cmd.remove(HbaPortCmd::ALPE);
        self.hba_port.cmd.set(cmd);
        self.start_cmd();
    }

    fn find_cmdslot(&self) -> Result<usize, SataDriveError> {
        let mut slots = self.hba_port.sact.get() | self.hba_port.ci.get();
        let num_of_slots = self.cap.number_of_slots().into();
        for i in 0..num_of_slots {
            if (slots & 1) == 0 {
                return Ok(i);
            }

            slots >>= 1;
        }
        return Err(SataDriveError::NoCmdSlot);
    }

    async fn run_command(
        &mut self,
        count: usize,
        buffer: &[DmaBuffer],
        command: DriveCommand,
    ) -> Result<(), SataDriveError> {
        let slot = self.find_cmdslot()?;
        let cmd_header = self.cmd_header(slot);

        let mut flags = cmd_header.flags.get();
        if command.is_write() {
            flags.intersects(HbaCmdHeaderFlags::W);
        } else {
            flags.remove(HbaCmdHeaderFlags::W);
        }

        flags.insert(HbaCmdHeaderFlags::P | HbaCmdHeaderFlags::C);
        flags.set_cfl(size_of::<FisRegInner<FisRegH2D>>() / size_of::<u32>());
        cmd_header.flags.set(flags);

        let length = ((count - 1) >> 5) + 1;
        cmd_header.prdtl.set(length as _);
        let cmdtbl = self.cmd_tbl(slot);
        for (buffer, i) in buffer.iter().zip(0..length) {
            assert!(buffer.start.as_u64() % 2 == 0);
            cmdtbl.get_prdt_entry(i).dba.set(buffer.start);
            cmdtbl.get_prdt_entry(i).set_dbc((buffer.size - 1) as u32);
            cmdtbl.get_prdt_entry(i).set_i(i == length - 1);
        }

        let mut cmdfis = cmdtbl.command_fis::<FisRegH2D>();
        cmdfis.control.set(0x00);
        cmdfis.icc.set(0x00);
        cmdfis.featurel.set(0x00);
        cmdfis.featureh.set(0x00);
        cmdfis.flags().set_command(true);
        cmdfis.command.set(command.to_ata());
        cmdfis.set_lba(command.sector());
        cmdfis.device.set(1 << 6);
        cmdfis.count.set(count as u16);

        self.hba_port.ci.set(1 << slot);
        while self.hba_port.tfd.get() & 0x80 | 0x08 == 1 {
            core::hint::spin_loop();
        }

        DriveAsync::new(self.hba_port, slot).await?;

        return Ok(());
    }
}

impl HbaMem {
    pub fn get_port(&mut self, port: usize) -> &'static mut HbaPort {
        unsafe { &mut *((self as *mut HbaMem).offset(1) as *mut HbaPort).add(port) }
    }
}

pub struct AhciDrive {
    port: Arc<Mutex<SataPort>>,
    identifier: Option<[u8; 512]>,
}

struct DriveAsync<'a> {
    port: &'a HbaPort,
    slot: usize,
}

impl<'a> DriveAsync<'a> {
    pub fn new(port: &'a HbaPort, slot: usize) -> Self {
        Self { port, slot }
    }
}

impl Future for DriveAsync<'_> {
    type Output = Result<(), SataDriveError>;

    fn poll(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        if self.port.is.get().contains(HbaPortIS::TFES) {
            return Poll::Ready(Err(SataDriveError::TaskFileError(self.port.serr.get())));
        }
        if self.port.ci.get() & (1 << self.slot) == 1 {
            return Poll::Pending;
        } else {
            return Poll::Ready(Ok(()));
        }
    }
}

impl AhciDrive {
    fn new(hba_port: &'static mut HbaPort, cap: HbaCapabilities) -> Self {
        let port = Mutex::new(SataPort {
            hba_port,
            clb: VirtAddr::new(0),
            fb: VirtAddr::new(0),
            ctba: [VirtAddr::new(0); 32],
            cap,
        });
        Self {
            port: Arc::new(port),
            identifier: None,
        }
    }

    async fn run_request(&self, request: &DmaRequest) -> Result<(), SataDriveError> {
        let mut count = request.count();
        let mut offset = 0;
        let mut current_sector = request.command.sector();

        while count > 0 {
            let this_count = count.min(128);
            self.port
                .lock()
                .run_command(
                    this_count,
                    &request.buffer[offset..],
                    request.command.replace_sector(current_sector),
                )
                .await?;
            count -= this_count; // 128 sector (65536 byte)
            offset += this_count >> 5; // 65536 byte per buffer
            current_sector += this_count as u64;
        }
        return Ok(());
    }

    pub async fn identify(&mut self) -> Result<(), SataDriveError> {
        let mut buf = [0u8; 512];
        let request =
            DmaRequest::new(1, DriveCommand::Identify).ok_or(SataDriveError::DmaRequest)?;
        self.run_request(&request).await?;
        request.copy_into(&mut buf);
        self.identifier = Some(buf);
        Ok(())
    }
}

impl Drive for AhciDrive {
    type Error = SataDriveError;

    async fn lba_end(&mut self) -> Result<u64, SataDriveError> {
        if let Some(identifier) = self.identifier {
            return Ok((u32::from_le_bytes(identifier[200..204].try_into().unwrap()) - 1).into());
        } else {
            self.identify().await?;
            return Ok((u32::from_le_bytes(
                self.identifier.unwrap()[200..204].try_into().unwrap(),
            ) - 1)
                .into());
        }
    }

    async fn read(
        &mut self,
        from_sector: u64,
        buffer: &mut [u8],
        count: usize,
    ) -> Result<(), Self::Error> {
        let request = DmaRequest::new(count, DriveCommand::Read(from_sector))
            .ok_or(SataDriveError::DmaRequest)?;
        self.run_request(&request).await?;
        request.copy_into(buffer);
        Ok(())
    }

    async fn write(
        &mut self,
        from_sector: u64,
        buffer: &[u8],
        count: usize,
    ) -> Result<(), Self::Error> {
        let mut request = DmaRequest::new(count, DriveCommand::Write(from_sector))
            .ok_or(SataDriveError::DmaRequest)?;
        request.copy_into_self(buffer);
        self.run_request(&request).await?;
        Ok(())
    }
}

pub struct AhciController {
    drives: [Option<AhciDrive>; 32],
    initialized: bool,
    hba: &'static mut HbaMem,
}

pub struct AhciDriver {
    inner: Mutex<AhciController>,
}

impl AhciDriver {
    pub fn get_contoller(&self) -> &Mutex<AhciController> {
        return &self.inner;
    }
}

impl PciDeviceHandle for AhciDriver {
    fn handles(&self, vendor_id: pci::Vendor, device_id: DeviceType) -> bool {
        matches!(
            (vendor_id, device_id),
            (Vendor::Intel | Vendor::Amd, DeviceType::SataController)
        )
    }

    fn start(&self, header: &pci::PciHeader) {
        if self.inner.lock().initialized() {
            log!(Warning, "Found two or more achi controller. currently the os support only one ahci controller, ignoring the other one.");
            return;
        }
        log!(Trace, "Initializing ahci driver (2nd phase)");
        log!(Trace, "Starting ahci driver");
        let abar = header.get_bar(5).expect("Failed to get ABAR for ahci");

        let (abar_address, _) = match abar {
            Bar::Memory32 { address, size, .. } => (address as u64, size as u64),
            Bar::Memory64 { address, size, .. } => (address, size),
            Bar::IO { .. } => panic!("ABAR is in port space somehow"),
        };

        memory_controller().lock().phy_map(
            ABAR_SIZE,
            abar_address,
            *ABAR_START,
            EntryFlags::PRESENT
                | EntryFlags::NO_CACHE
                | EntryFlags::WRITABLE
                | EntryFlags::WRITE_THROUGH,
        );

        self.inner.lock().probe_port();
    }
}

impl AhciController {
    pub fn new() -> Self {
        Self {
            drives: [const { None }; 32],
            initialized: false,
            hba: unsafe { &mut *((*ABAR_START as *mut u8) as *mut HbaMem) },
        }
    }

    pub fn initialized(&self) -> bool {
        self.initialized
    }
    pub fn probe_port(&mut self) {
        self.initialized = true;
        let pi = self.hba.pi.get();

        if self.hba.bohc.get() & 2 == 0 {
            self.hba.bohc.set(self.hba.bohc.get() | 0b10);
            let mut spin = 0;
            while self.hba.bohc.get() & 1 != 0 && spin < 50000 {
                spin += 1;
            }
            if self.hba.bohc.get() & 1 != 0 {
                self.hba.bohc.set(2);
                self.hba.bohc.set(self.hba.bohc.get() | 8);
            }
        }

        for i in 0..32 {
            if pi.get_bit(i) {
                let drive = AhciDrive::new(self.hba.get_port(i), self.hba.cap.get());
                let dt = drive.port.lock().check_type();
                if let Some(dt) = dt {
                    match dt {
                        AhciDriveType::Sata => {
                            drive.port.lock().rebase();
                            log!(Info, "Found sata drive on port {}", i);
                            self.drives[i] = Some(drive);
                        }
                        dt => log!(
                            Warning,
                            "AHCI drive detected. but not support on port: {}, drive type: {}",
                            i,
                            dt
                        ),
                    }
                }
            }
        }
    }

    pub fn get_drive(&mut self, id: usize) -> Result<&mut AhciDrive, SataDriveError> {
        if let Some(drive) = self.drives.get_mut(id) {
            if let Some(value) = drive {
                return Ok(value);
            } else {
                return Err(SataDriveError::DriveNotFound(id));
            }
        } else {
            return Err(SataDriveError::DriveNotFound(id));
        }
    }
}

pub fn get_ahci() -> &'static Arc<AhciDriver> {
    return DRIVER.get().expect("AHCI driver not initialized");
}

pub fn init() {
    log!(Trace, "Initializing ahci driver (1st phase)");
    DRIVER.call_once(|| {
        Arc::new(AhciDriver {
            inner: Mutex::new(AhciController::new()),
        })
    });
    log!(Trace, "Registering ahci driver");
    register_driver(get_ahci().clone());
}
