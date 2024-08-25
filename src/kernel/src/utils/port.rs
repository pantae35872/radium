use core::arch::asm;

pub struct Port16Bit {
    portnumber: u16,
}

impl Port16Bit {
    pub fn new(portnumber: u16) -> Self {
        Self { portnumber }
    }

    pub unsafe fn write(&self, data: u16) {
        asm!("outw %ax, %dx", 
                in("ax") data,
                in("dx") self.portnumber,
                options(att_syntax));
    }

    pub unsafe fn read(&self) -> u16 {
        let mut result: u16;
        asm!("inw %dx, %ax",
                 out("ax") result,
                 in("dx") self.portnumber,
                 options(att_syntax));
        result
    }
}

pub struct Port8Bit {
    portnumber: u16,
}

impl Port8Bit {
    pub fn new(portnumber: u16) -> Self {
        Self { portnumber }
    }

    pub unsafe fn write(&self, data: u8) {
        asm!("outb %al, %dx", 
                in("al") data,
                in("dx") self.portnumber,
                options(att_syntax));
    }

    pub unsafe fn read(&self) -> u8 {
        let mut result: u8;
        asm!("inb %dx, %al",
                out("al") result,
                in("dx") self.portnumber, 
                options(att_syntax));
        return result;
    }
}

pub struct Port32Bit {
    portnumber: u16,
}

impl Port32Bit {
    pub fn new(portnumber: u16) -> Self {
        Self { portnumber }
    }

    pub unsafe fn write(&self, data: u32) {
        asm!("outl %eax, %dx",
                in("eax") data,
                in("dx") self.portnumber,
                options(att_syntax));
    }

    pub unsafe fn read(&self) -> u32 {
        let mut result: u32;
        asm!("inl %dx, %eax",
                out("eax") result,
                in("dx") self.portnumber,
                options(att_syntax));
        return result;
    }
}
