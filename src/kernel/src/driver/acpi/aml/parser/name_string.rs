use core::iter;

use alloc::vec;
use alloc::vec::Vec;

use crate::driver::acpi::aml::namespace::{AmlName, NameComponent, NameSeg};
use crate::driver::acpi::aml::parser::choose;
use crate::driver::acpi::aml::parser::opcode::NULL_NAME;
use crate::driver::acpi::aml::{AmlContext, AmlError};

use super::opcode::{DUAL_NAME_PREFIX, MULTI_NAME_PREFIX, PREFIX_CHAR, ROOT_CHAR, opcode};
use super::{Parser, byte_data, either, pair, zero_or_more};

pub fn name_string<'a, 'c>() -> impl Parser<'a, 'c, AmlName>
where
    'c: 'a,
{
    either(
        root_char().then(name_path().arced()).map(|e| AmlName(iter::once(NameComponent::Root).chain(e).collect())),
        prefix_path()
            .and_then(|n| name_path().map(move |e| (n, e)))
            .map(|(e, n)| AmlName(iter::repeat(NameComponent::Prefix).take(e).chain(n).collect())),
    )
}

fn prefix_path<'a, 'c>() -> impl Parser<'a, 'c, usize>
where
    'c: 'a,
{
    zero_or_more(opcode(PREFIX_CHAR)).map(|e| e.len())
}

fn name_path<'a, 'c>() -> impl Parser<'a, 'c, Vec<NameComponent>>
where
    'c: 'a,
{
    choose!(name_seg().map(|e| vec![e]), dual_name(), multiname(), opcode(NULL_NAME).map(|_| vec![]),)
}

fn multiname<'a, 'c>() -> impl Parser<'a, 'c, Vec<NameComponent>>
where
    'c: 'a,
{
    opcode(MULTI_NAME_PREFIX).then(
        byte_data
            .and_then(|e| {
                move |input, context| {
                    (0..e).try_fold((input, context, Vec::new()), |(input, context, mut result), _| {
                        name_seg().parse(input, context).map(|(next_input, next_context, item)| {
                            result.push(item);
                            (next_input, next_context, result)
                        })
                    })
                }
            })
            .arced(),
    )
}

fn dual_name<'a, 'c>() -> impl Parser<'a, 'c, Vec<NameComponent>>
where
    'c: 'a,
{
    opcode(DUAL_NAME_PREFIX).then(pair(name_seg(), name_seg()).map(|(a, b)| vec![a, b]).arced())
}

fn name_seg<'a, 'c>() -> impl Parser<'a, 'c, NameComponent>
where
    'c: 'a,
{
    move |input: &'a [u8], context: &'c mut AmlContext| match input
        .get(0..4)
        .and_then(|e| NameSeg::new_bytes(<[u8; 4]>::try_from(e).unwrap()).map(|e| e.into()))
    {
        Some(e) => Ok((&input[4..], context, e)),
        None => Err((input, context, AmlError::ParserError.into())),
    }
}

fn root_char<'a, 'c>() -> impl Parser<'a, 'c, ()>
where
    'c: 'a,
{
    opcode(ROOT_CHAR)
}

#[cfg(test)]
mod tests {
    use alloc::{string::String, vec::Vec};

    use crate::driver::acpi::aml::{
        TestHandle,
        parser::{parser_err, parser_ok},
    };

    use super::*;
    use core::str::FromStr;
    #[test_case]
    fn relative_name_string_test() {
        let mut context = AmlContext::new(TestHandle);
        parser_ok!(name_string(), [b'_', b'A', b'B', b'_'], &mut context, AmlName::from_str("_AB_").unwrap());
        parser_ok!(name_string(), [b'^', b'E', b'A', b'B', b'_'], &mut context, AmlName::from_str("^EAB_").unwrap());
        parser_ok!(
            name_string(),
            [b'^', b'^', b'^', 0x2E, b'F', b'A', b'B', b'G', b'_', b'S', b'B', b'_'],
            &mut context,
            AmlName::from_str("^^^FABG._SB_").unwrap()
        );
    }

    #[test_case]
    fn absolute_name_string_test() {
        let mut context = AmlContext::new(TestHandle);
        parser_ok!(name_string(), [0x5C, b'_', b'S', b'B', b'_'], &mut context, AmlName::from_str("\\_SB_").unwrap());
    }

    #[test_case]
    fn null_name_string_test() {
        let mut context = AmlContext::new(TestHandle);
        parser_ok!(name_string(), [0x00], &mut context, AmlName::null_name());
    }

    #[test_case]
    fn dual_name_path_test() {
        let mut context = AmlContext::new(TestHandle);
        parser_ok!(
            name_string(),
            [0x2E, b'E', b'A', b'D', b'E', b'E', b'4', b'3', b'2'],
            &mut context,
            AmlName::from_str("EADE.E432").unwrap(),
        );
        parser_err!(name_path(), [0x2E, b'E', b'A', b'D', b'E'], &mut context);
    }

    #[test_case]
    fn multi_name_path_test() {
        let mut context = AmlContext::new(TestHandle);
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
        parser_ok!(name_string(), multiname_test, &mut context, AmlName::from_str(&name).unwrap());

        parser_err!(name_string(), [0x2F, 3], &mut context);
    }

    #[test_case]
    fn single_name_path_test() {
        let mut context = AmlContext::new(TestHandle);
        parser_ok!(name_string(), [b'_', b'A', b'D', b'E'], &mut context, AmlName::from_str("_ADE").unwrap());
        parser_ok!(name_string(), [b'_', b'A', b'2', b'1'], &mut context, AmlName::from_str("_A21").unwrap());
        parser_err!(name_string(), [b'e', b'A', b'D', b'E'], &mut context);
        parser_err!(name_string(), [b'1', b'A', b'D', b'E'], &mut context);
    }
}
