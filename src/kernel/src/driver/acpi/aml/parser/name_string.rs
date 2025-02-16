use core::iter;

use alloc::vec;
use alloc::vec::Vec;

use crate::choose;
use crate::driver::acpi::aml::namespace::{AmlName, NameComponent, NameSeg};
use crate::driver::acpi::aml::parser::opcode::NULL_NAME;

use super::opcode::{opcode, DUAL_NAME_PREFIX, MULTI_NAME_PREFIX, PREFIX_CHAR, ROOT_CHAR};
use super::{byte_data, either, pair, zero_or_more, Parser};

fn name_string<'a>() -> impl Parser<'a, AmlName> {
    either(
        root_char()
            .then(name_path().arced())
            .map(|e| AmlName(iter::once(NameComponent::Root).chain(e).collect())),
        prefix_path()
            .and_then(|n| name_path().map(move |e| (n, e)))
            .map(|(e, n)| {
                AmlName(
                    iter::repeat(NameComponent::Prefix)
                        .take(e)
                        .chain(n)
                        .collect(),
                )
            }),
    )
}

fn prefix_path<'a>() -> impl Parser<'a, usize> {
    zero_or_more(opcode(PREFIX_CHAR)).map(|e| e.len())
}

fn name_path<'a>() -> impl Parser<'a, Vec<NameComponent>> {
    choose!(
        name_seg().map(|e| vec![e]),
        dual_name(),
        multiname(),
        opcode(NULL_NAME).map(|_| vec![]),
    )
}

fn multiname<'a>() -> impl Parser<'a, Vec<NameComponent>> {
    opcode(MULTI_NAME_PREFIX).then(
        byte_data
            .and_then(|e| {
                move |input| {
                    (0..e).try_fold((input, Vec::new()), |(input, mut result), _| {
                        name_seg().parse(input).map(|(next_input, item)| {
                            result.push(item);
                            (next_input, result)
                        })
                    })
                }
            })
            .arced(),
    )
}

fn dual_name<'a>() -> impl Parser<'a, Vec<NameComponent>> {
    opcode(DUAL_NAME_PREFIX).then(
        pair(name_seg(), name_seg())
            .map(|(a, b)| vec![a, b])
            .arced(),
    )
}

fn name_seg<'a>() -> impl Parser<'a, NameComponent> {
    move |input: &'a [u8]| {
        input
            .get(0..4)
            .and_then(|e| NameSeg::new_bytes(<[u8; 4]>::try_from(e).unwrap()).map(|e| e.into()))
            .map(|e| (&input[4..], e))
            .ok_or(input)
    }
}

fn root_char<'a>() -> impl Parser<'a, ()> {
    opcode(ROOT_CHAR)
}

#[cfg(test)]
mod tests {
    use crate::{parser_err, parser_ok};
    use alloc::{string::String, vec::Vec};

    use super::*;
    use core::str::FromStr;
    #[test_case]
    fn relative_name_string_test() {
        parser_ok!(
            name_string(),
            [b'_', b'A', b'B', b'_'],
            AmlName::from_str("_AB_").unwrap()
        );
        parser_ok!(
            name_string(),
            [b'^', b'E', b'A', b'B', b'_'],
            AmlName::from_str("^EAB_").unwrap()
        );
        parser_ok!(
            name_string(),
            [b'^', b'^', b'^', 0x2E, b'F', b'A', b'B', b'G', b'_', b'S', b'B', b'_'],
            AmlName::from_str("^^^FABG._SB_").unwrap()
        );
    }

    #[test_case]
    fn absolute_name_string_test() {
        parser_ok!(
            name_string(),
            [0x5C, b'_', b'S', b'B', b'_'],
            AmlName::from_str("\\_SB_").unwrap()
        );
    }

    #[test_case]
    fn null_name_string_test() {
        parser_ok!(name_string(), [0x00], AmlName::null_name());
    }

    #[test_case]
    fn dual_name_string_test() {
        parser_ok!(
            name_string(),
            [0x2E, b'E', b'A', b'D', b'E', b'E', b'4', b'3', b'2'],
            AmlName::from_str("EADE.E432").unwrap(),
        );
        parser_err!(name_path(), [0x2E, b'E', b'A', b'D', b'E']);
    }

    #[test_case]
    fn multi_name_path_test() {
        let count = 4;
        let mut multiname_test = vec![0x2F, count];
        for _ in 0..(count - 1) {
            multiname_test.extend_from_slice(&[b'A', b'E', b'D', b'G']);
        }
        multiname_test.extend_from_slice(&[b'I', b'F', b'F', b'E']);
        let name = multiname_test
            .iter()
            .skip(2)
            .map(|e| *e as char)
            .array_chunks::<4>()
            .map(|chunk| chunk.iter().collect::<String>())
            .collect::<Vec<String>>()
            .join(".");
        parser_ok!(
            name_string(),
            multiname_test,
            AmlName::from_str(&name).unwrap()
        );

        parser_err!(name_string(), [0x2F, 3]);
    }

    #[test_case]
    fn name_path_test() {
        //assert_eq!(
        //    name_path().parse(&[b'_', b'A', b'D', b'E']),
        //    Ok((vec![].as_slice(), Some(vec![['_', 'A', 'D', 'E']])))
        //);
        //assert_eq!(
        //    name_path().parse(&[b'_', b'A', b'2', b'1']),
        //    Ok((vec![].as_slice(), Some(vec![['_', 'A', '2', '1']])))
        //);
        //assert_eq!(
        //    name_path().parse(&[b'e', b'A', b'D', b'E']),
        //    Err(vec![b'e', b'A', b'D', b'E'].as_slice()),
        //);
        //assert_eq!(
        //    name_path().parse(&[b'1', b'A', b'D', b'E']),
        //    Err(vec![b'1', b'A', b'D', b'E'].as_slice()),
        //);
    }
}
