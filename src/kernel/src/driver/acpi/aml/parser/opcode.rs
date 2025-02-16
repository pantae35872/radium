use super::Parser;

// Name String
pub const NULL_NAME: u8 = 0x00;
pub const DUAL_NAME_PREFIX: u8 = 0x2E;
pub const MULTI_NAME_PREFIX: u8 = 0x2F;
pub const ROOT_CHAR: u8 = b'\\';
pub const PREFIX_CHAR: u8 = b'^';

// Name Space Modifier
pub const DEF_ALIAS: u8 = 0x06;
pub const DEF_NAME: u8 = 0x08;
pub const DEF_SCOPE: u8 = 0x10;

pub const EXT_OPCODE_PREFIX: u8 = 0x5b;

pub fn opcode<'a>(opcode: u8) -> impl Parser<'a, ()> {
    move |input: &'a [u8]| match input.first() {
        Some(&byte) if byte == opcode => Ok((&input[1..], ())),
        None | Some(_) => Err(input),
    }
}

pub fn ext_opcode<'a>(ext_opcode: u8) -> impl Parser<'a, ()> {
    opcode(EXT_OPCODE_PREFIX).then(opcode(ext_opcode).arced())
}
