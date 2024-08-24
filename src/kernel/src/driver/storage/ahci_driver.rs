use core::alloc::Layout;
use core::error::Error;
use core::fmt::Display;
use core::mem::size_of;
use core::ptr::{self, write_bytes};
use core::{u32, usize};

use alloc::alloc::alloc;
use alloc::vec::Vec;
use bit_field::BitField;
use bitfield_struct::bitfield;
use conquer_once::spin::OnceCell;
use spin::Mutex;
use x86_64::{PhysAddr, VirtAddr};

use crate::driver::pci;
use crate::memory::paging::Page;
use crate::memory::{AreaFrameAllocator, Frame};
use crate::utils::VolatileCell;
use crate::{get_physical, println, EntryFlags, ACTIVE_TABLE};

use super::Drive;

pub static DRIVER: OnceCell<Mutex<AhciController>> = OnceCell::uninit();

pub const ABAR_START: usize = 0xFFFFFFFF00000000;
pub const ABAR_SIZE: usize = size_of::<HbaMem>();

#[derive(Debug)]
pub enum SataDriveError {
    NoCmdSlot,
    TaskFileError(u32),
    DriveNotFound(usize),
}

impl Display for SataDriveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoCmdSlot => write!(f, "Cannot find free command list entry"),
            Self::TaskFileError(serr) => {
                write!(f, "Execute command with task file error: {}", serr)
            }
            Self::DriveNotFound(id) => write!(f, "Trying to get drive with id: {}", id),
        }
    }
}

impl Error for SataDriveError {}

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

#[bitfield(u8, order = Lsb)]
pub struct FisRegH2DByte1 {
    #[bits(4)]
    pmport: u8,
    #[bits(3)]
    rsv: u8,
    #[bits(1)]
    command: bool,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
enum FisType {
    RegH2D = 0x27,
}

#[derive(Debug)]
#[repr(C)]
pub struct FisRegH2D {
    // DWORD 0
    fis_type: VolatileCell<FisType>, // FIS_TYPE_REG_H2D
    byte_1: FisRegH2DByte1,
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
    countl: VolatileCell<u8>, // Count register, 7:0
    counth: VolatileCell<u8>,
    icc: VolatileCell<u8>,     // Isochronous command completion
    control: VolatileCell<u8>, // Control register

    // DWORD 4
    rsv1: [VolatileCell<u8>; 4], // Reserved
}

impl FisRegH2D {
    pub fn set_command(&mut self, value: &bool) {
        self.byte_1.set_command(*value);
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct HbaMem {
    // 0x00 - 0x2B, Generic Host Control
    cap: VolatileCell<u32>,     // 0x00, Host capability
    ghc: VolatileCell<u32>,     // 0x04, Global host control
    is: VolatileCell<u32>,      // 0x08, Interrupt status
    pi: VolatileCell<u32>,      // 0x0C, Port implemented
    vs: VolatileCell<u32>,      // 0x10, Version
    ccc_ctl: VolatileCell<u32>, // 0x14, Command completion coalescing control
    ccc_pts: VolatileCell<u32>, // 0x18, Command completion coalescing ports
    em_loc: VolatileCell<u32>,  // 0x1C, Enclosure management location
    em_ctl: VolatileCell<u32>,  // 0x20, Enclosure management control
    cap2: VolatileCell<u32>,    // 0x24, Host capabilities extended
    bohc: VolatileCell<u32>,    // 0x28, BIOS/OS handoff control and status

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
pub struct HbaCmdTbl {
    // 0x00
    cfis: [VolatileCell<u8>; 64], // Command FIS

    // 0x40
    acmd: [VolatileCell<u8>; 16], // ATAPI command, 12 or 16 bytes

    // 0x50
    _reserved: [VolatileCell<u8>; 48], // Reserved

    // 0x80
    prdt_entry: [HbaPRDTEntry; 1], // Physical region descriptor table entries, 0 ~ 65535
}

impl HbaCmdTbl {
    pub fn get_prdt_entry(&mut self, index: usize) -> &mut HbaPRDTEntry {
        unsafe { &mut *self.prdt_entry.as_mut_ptr().add(index) }
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
    serr: VolatileCell<u32>,             // 0x30, SATA error (SCR1:SError)
    sact: VolatileCell<u32>,             // 0x34, SATA active (SCR3:SActive)
    ci: VolatileCell<u32>,               // 0x38, command issue
    sntf: VolatileCell<u32>,             // 0x3C, SATA notification (SCR4:SNotification)
    fbs: VolatileCell<u32>,              // 0x40, FIS-based switch control
    _reserved1: [VolatileCell<u32>; 11], // 0x44 ~ 0x6F, Reserved
    vendor: [VolatileCell<u32>; 4],      // 0x70 ~ 0x7F, vendor specific
}

pub struct AhciPort {
    hba_port: &'static mut HbaPort,
    clb: VirtAddr,
    fb: VirtAddr,
    ctba: [VirtAddr; 32],
    cap: usize,
}

impl AhciPort {
    fn cmd_header(&self, slot: usize) -> &mut HbaCmdHeader {
        unsafe { &mut *(self.clb.as_mut_ptr::<HbaCmdHeader>().byte_add(slot)) }
    }

    fn cmd_tbl(&self) -> Result<&mut HbaCmdTbl, SataDriveError> {
        let slot = self.find_cmdslot()?;
        return Ok(unsafe { &mut *(self.ctba[slot].as_mut_ptr::<HbaCmdTbl>()) });
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
        self.hba_port.clb.set(get_physical(virt_clb).unwrap());
        let virt_fb: VirtAddr =
            unsafe { VirtAddr::new(alloc(Layout::from_size_align(0xFF, 256).unwrap()) as u64) };
        unsafe {
            write_bytes(virt_fb.as_mut_ptr::<u8>(), 0, 0xFF);
        }
        self.hba_port.fb.set(get_physical(virt_fb).unwrap());
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
            cmdheader[i].ctba.set(get_physical(virt_ctba).unwrap());
        }
        self.hba_port.ie.set(HbaPortIE::all());
        self.start_cmd();
    }

    fn find_cmdslot(&self) -> Result<usize, SataDriveError> {
        let mut slots = self.hba_port.sact.get() | self.hba_port.ci.get();
        let num_of_slots = (self.cap & 0x0F00) >> 8;
        for i in 0..num_of_slots {
            if (slots & 1) == 0 {
                return Ok(i);
            }

            slots >>= 1;
        }
        return Err(SataDriveError::NoCmdSlot);
    }
}

impl HbaMem {
    pub fn get_port(&mut self, port: usize) -> &'static mut HbaPort {
        unsafe { &mut *((self as *mut HbaMem).offset(1) as *mut HbaPort).add(port) }
    }
}

pub struct AhciDrive {
    port: Mutex<AhciPort>,
    identifier: Option<[u8; 512]>,
}

impl AhciDrive {
    pub fn new(hba_port: &'static mut HbaPort, cap: usize) -> Self {
        let port = Mutex::new(AhciPort {
            hba_port,
            clb: VirtAddr::new(0),
            fb: VirtAddr::new(0),
            ctba: [VirtAddr::new(0); 32],
            cap,
        });
        Self {
            port,
            identifier: None,
        }
    }

    // TODO: Ensure that buffer is word align (2 byte align)
    fn run_command(
        &mut self,
        lba: u64,
        original_count: usize,
        buffer: &[u8],
        command: u8,
        lba_mode: bool,
    ) -> Result<(), SataDriveError> {
        let port = self.port.lock();
        let mut count = original_count;
        let slot = port.find_cmdslot()?;
        let cmd_header = port.cmd_header(slot);

        let mut flags = cmd_header.flags.get();
        if command == 0x35 {
            flags.intersects(HbaCmdHeaderFlags::W);
        } else {
            flags.remove(HbaCmdHeaderFlags::W);
        }

        flags.insert(HbaCmdHeaderFlags::P | HbaCmdHeaderFlags::C);
        flags.set_cfl(size_of::<FisRegH2D>() / size_of::<u32>());
        cmd_header.flags.set(flags);

        cmd_header.prdtl.set((((count - 1) >> 4) + 1) as u16);
        let cmdtbl = port.cmd_tbl()?;
        let mut buf_address = get_physical(VirtAddr::new(buffer.as_ptr() as u64)).unwrap();
        assert!(buf_address.is_aligned(2u64));
        let mut i: usize = 0;
        while i < cmd_header.prdtl.get() as usize - 1 {
            cmdtbl.get_prdt_entry(i).dba.set(buf_address);
            cmdtbl.get_prdt_entry(i).set_dbc(8 * 1024 - 1);
            cmdtbl.get_prdt_entry(i).set_i(false);
            buf_address += (8 * 1024) as u64;
            count -= 16;
            i += 1;
        }

        cmdtbl.get_prdt_entry(i).dba.set(buf_address);
        cmdtbl.get_prdt_entry(i).set_dbc((count << 9) as u32 - 1);
        cmdtbl.get_prdt_entry(i).set_i(true);

        let cmdfis = unsafe { &mut *(cmdtbl.cfis.as_mut_ptr() as *mut FisRegH2D) };
        unsafe { core::ptr::write_bytes(cmdfis as *mut FisRegH2D, 0, 1) }
        cmdfis.control.set(0x00);
        cmdfis.icc.set(0x00);
        cmdfis.featurel.set(0x00);
        cmdfis.featureh.set(0x00);
        cmdfis.fis_type.set(FisType::RegH2D);
        cmdfis.set_command(&true);
        cmdfis.command.set(command);
        if lba_mode {
            cmdfis.lba0.set(lba as u8);
            cmdfis.lba1.set((lba >> 8) as u8);
            cmdfis.lba2.set((lba >> 16) as u8);
            cmdfis.device.set(1 << 6);
            cmdfis.lba3.set((lba >> 24) as u8);
            cmdfis.lba4.set((lba >> 32) as u8);
            cmdfis.lba5.set((lba >> 40) as u8);
            cmdfis.countl.set((original_count & 0xFF) as u8);
            cmdfis.counth.set((original_count >> 8) as u8);
        } else {
            cmdfis.device.set(0);
        }

        port.hba_port.ci.set(1 << slot);
        while port.hba_port.tfd.get() & 0x80 | 0x08 == 1 {
            core::hint::spin_loop();
        }
        while port.hba_port.ci.get() & (1 << slot) == 1 {
            if port.hba_port.is.get().contains(HbaPortIS::TFES) {
                return Err(SataDriveError::TaskFileError(port.hba_port.serr.get()));
            }
        }

        if port.hba_port.is.get().contains(HbaPortIS::TFES) {
            return Err(SataDriveError::TaskFileError(port.hba_port.serr.get()));
        }

        return Ok(());
    }

    pub fn identify(&mut self) -> Result<(), SataDriveError> {
        let buf = [0u8; 512];
        self.run_command(0, 1, &buf, 0xEC, false)?;
        self.identifier = Some(buf);
        Ok(())
    }
}

impl Drive for AhciDrive {
    type Error = SataDriveError;
    fn lba_end(&self) -> u64 {
        if let Some(identifier) = self.identifier {
            return (u32::from_le_bytes(identifier[200..204].try_into().unwrap()) - 1).into();
        } else {
            panic!("Please identify the drive before accessing it");
        }
    }

    fn read(
        &mut self,
        from_sector: u64,
        buffer: &mut [u8],
        count: usize,
    ) -> Result<(), Self::Error> {
        return self.run_command(from_sector, count, buffer, 0x25, true);
    }

    fn write(&mut self, from_sector: u64, buffer: &[u8], count: usize) -> Result<(), Self::Error> {
        return self.run_command(from_sector, count, buffer, 0x35, true);
    }
}

pub struct AhciController {
    drives: Vec<AhciDrive>,
    hba: &'static mut HbaMem,
}

impl AhciController {
    pub fn new() -> Self {
        Self {
            drives: Vec::new(),
            hba: unsafe { &mut *((ABAR_START as *mut u8) as *mut HbaMem) },
        }
    }

    pub fn probe_port(&mut self) {
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
                let mut drive = AhciDrive::new(self.hba.get_port(i), self.hba.cap.get() as usize);
                let dt = drive.port.lock().check_type();
                if let Some(dt) = dt {
                    match dt {
                        AhciDriveType::Sata => {
                            drive.port.lock().rebase();
                            drive.identify().expect("identify drive failed");
                            self.drives.push(drive);
                        }
                        dt => println!("Drive not support: {}", dt),
                    }
                }
            }
        }
    }

    pub fn get_drive(&mut self, id: usize) -> Result<&mut AhciDrive, SataDriveError> {
        if let Some(drive) = self.drives.get_mut(id) {
            return Ok(drive);
        } else {
            return Err(SataDriveError::DriveNotFound(id));
        }
    }

    pub fn get_ahci_address() -> Option<u64> {
        let mut pci = pci::DRIVER
            .get()
            .expect("PCI driver is not initialize")
            .lock();
        let mut vendor: u32;
        let mut device: u32;
        for bus in 0..256 {
            for slot in 0..32 {
                vendor = pci.read_config(bus, slot, 0, 0x00 | 0x0);
                device = pci.read_config(bus, slot, 0, 0x00 | 0x02);
                if (vendor == 0x8086 && device == 0x2922)
                    || (vendor == 0x8086 && device == 0x2829)
                    || (vendor == 0x8086 && device == 0xa103)
                {
                    let command = pci.read(bus, slot, 0, 0x04);
                    pci.write(bus, slot, 0, 0x4, command | 1 << 1 | 1 << 2);

                    let mut interrupt = pci.read(bus, slot, 0, 0x3C);
                    interrupt.set_bits(0..8, 10);
                    pci.write(bus, slot, 0, 0x3C, interrupt);

                    return Some(pci.read(bus, slot, 0, 0x24).into());
                }
            }
        }
        None
    }
}

pub fn init(area_frame_allocator: &mut AreaFrameAllocator) {
    let abar_start_page = Page::containing_address(ABAR_START);
    let abar_end_page = Page::containing_address(ABAR_START + ABAR_SIZE);
    let abar_address = AhciController::get_ahci_address();
    if let Some(abar_address) = abar_address {
        for (page, frame) in
            Page::range_inclusive(abar_start_page, abar_end_page).zip(Frame::range_inclusive(
                Frame::containing_address(abar_address as usize),
                Frame::containing_address(abar_address as usize + ABAR_SIZE),
            ))
        {
            ACTIVE_TABLE
                .get()
                .expect("incorrect order of initializeation from ahci")
                .lock()
                .map_to(
                    page,
                    frame,
                    EntryFlags::PRESENT
                        | EntryFlags::NO_CACHE
                        | EntryFlags::WRITABLE
                        | EntryFlags::WRITE_THROUGH,
                    area_frame_allocator,
                );
        }
        DRIVER.init_once(|| {
            let mut controller = AhciController::new();
            controller.probe_port();
            Mutex::from(controller)
        });
    }
}
