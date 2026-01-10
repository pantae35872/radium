//! Writer implementation for ISO-9660
//!
//! Derived from:
//! http://wiki.osdev.org/ISO_9660#Numerical_formats

use chrono::{Datelike, Local, Timelike};

#[derive(Debug, Clone, Default)]
pub struct TypeWriter {
    buffer: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Endian {
    #[default]
    Little,
    Big,
    Both,
}

macro_rules! impl_write_type {
    ($ty: ty) => {
        paste::paste! {
            pub fn [< write_$ty >](&mut self, value: $ty, endian: Endian) -> &mut Self {
                match endian {
                    Endian::Little => self.write_bytes(&value.to_le_bytes()),
                    Endian::Big => self.write_bytes(&value.to_be_bytes()),
                    Endian::Both => self.write_bytes(&[value.to_le_bytes(), value.to_be_bytes()].concat()),
                };
                self
            }
        }
    };
}

impl TypeWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.buffer.extend_from_slice(bytes);
        self
    }

    pub fn write_u8(&mut self, value: u8) -> &mut Self {
        self.write_bytes(&value.to_le_bytes());
        self
    }

    pub fn write_i8(&mut self, value: i8) -> &mut Self {
        self.write_bytes(&value.to_le_bytes());
        self
    }

    pub fn write_date_time(&mut self, datetime: &DateTime) -> &mut Self {
        self.write_str(&datetime.time);
        self.write_u8(datetime.timezone);
        self
    }

    pub fn write_str_padded(&mut self, value: &impl IsoStr, length: usize) -> &mut Self {
        let str_bytes = value.as_ref().as_bytes();
        let before_len = self.len();
        if str_bytes.len() > length {
            self.write_bytes(&str_bytes[0..length]);
        } else {
            self.write_bytes(&str_bytes);
            for _ in str_bytes.len()..length {
                self.buffer.push(0x20);
            }
        }
        assert_eq!(before_len + length, self.len());
        self
    }

    pub fn write_str(&mut self, value: &impl IsoStr) -> &mut Self {
        self.write_bytes(value.as_ref().as_bytes());
        self
    }

    impl_write_type!(u16);
    impl_write_type!(i16);
    impl_write_type!(u32);
    impl_write_type!(i32);

    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    pub fn buffer_owned(self) -> Vec<u8> {
        self.buffer
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTime {
    time: IsoStrD,
    timezone: u8,
}

impl From<chrono::DateTime<Local>> for DateTime {
    fn from(time: chrono::DateTime<Local>) -> Self {
        let timezone = (time.offset().local_minus_utc() / (15 * 60)) as i8 as u8;
        let time = IsoStrD::new(format!(
            "{:0>4}{:0>2}{:0>2}{:0>2}{:0>2}{:0>2}{:0>2}",
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
            time.second(),
            0, // FIXME: WHY, Hundredths of a second from 0 to 99.
        ));
        assert_eq!(time.as_ref().len(), 16);
        Self { time, timezone }
    }
}

impl DateTime {
    pub fn empty() -> Self {
        Self { time: IsoStrD::new("0000000000000000"), timezone: 0 }
    }

    pub fn now() -> Self {
        Self::from(Local::now())
    }
}

pub trait IsoStr: AsRef<str> {}

impl IsoStr for IsoStrD {}
impl IsoStr for IsoStrA {}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct IsoStrD(String);

impl IsoStrD {
    pub fn new<T: AsRef<str>>(str: T) -> Self {
        let ascii = str.as_ref().chars().map(|c| if c.is_ascii() { c as u8 } else { b'_' });
        let converted = ascii
            .map(|c| match c {
                b'a'..=b'z' => c - 32,
                b'A'..=b'Z' => c,
                b'0'..=b'9' => c,
                b' ' => c,
                b'_' => c,
                _ => b'_',
            })
            .map(|c| c as char)
            .collect();
        Self(converted)
    }
}

impl AsRef<str> for IsoStrD {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct IsoStrA(String);

impl IsoStrA {
    pub fn new<T: AsRef<str>>(str: T) -> Self {
        let ascii = str.as_ref().chars().map(|c| if c.is_ascii() { c as u8 } else { b'_' });
        let converted = ascii
            .map(|c| match c {
                b' ' => c,
                b'a'..=b'z' => c - 32,
                b'A'..=b'Z'
                | b'0'..=b'9'
                | b'!'
                | b'"'
                | b'%'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b'-'
                | b'.'
                | b'/'
                | b':'
                | b';'
                | b'<'
                | b'='
                | b'>'
                | b'?' => c,
                _ => b'?',
            })
            .map(|c| c as char)
            .collect();

        Self(converted)
    }
}

impl AsRef<str> for IsoStrA {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
