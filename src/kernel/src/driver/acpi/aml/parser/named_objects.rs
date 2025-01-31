use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::choose;

use super::{byte_data, either, match_bytes, pair, zero_or_more, Parser};

type NameSeg = [char; 4];

#[derive(Debug, PartialEq, Eq)]
enum NameString {
    Absolute(Vec<NameSeg>),        // Starts with `\`
    Relative(usize, Vec<NameSeg>), // `usize` represents the number of `^`
}

fn name_string<'a>() -> impl Parser<'a, NameString> {
    either(
        root_char()
            .then(name_path().arced())
            .map(|e| NameString::Absolute(e.unwrap_or(Vec::new()))),
        prefix_path()
            .and_then(|n| name_path().map(move |e| (n, e.unwrap_or(Vec::new()))))
            .map(|(e, n)| NameString::Relative(e, n)),
    )
}

fn prefix_path<'a>() -> impl Parser<'a, usize> {
    zero_or_more(match_bytes(&[0x5E])).map(|e| e.len())
}

fn name_path<'a>() -> impl Parser<'a, Option<Vec<NameSeg>>> {
    choose!(
        name_seg().map(|e| Some(vec![e])),
        dual_name().map(|e| Some(e)),
        multiname().map(|e| Some(e)),
        match_bytes(&[0x00]).map(|_| None),
    )
}

fn multiname<'a>() -> impl Parser<'a, Vec<NameSeg>> {
    match_bytes(&[0x2F]).then(
        byte_data
            .and_then(|e| {
                move |mut input| {
                    let mut result = Vec::new();
                    for _ in 0..e {
                        match name_seg().parse(input) {
                            Ok((next_input, item)) => {
                                input = next_input;
                                result.push(item);
                            }
                            Err(err) => return Err(err),
                        }
                    }
                    Ok((input, result))
                }
            })
            .arced(),
    )
}

fn dual_name<'a>() -> impl Parser<'a, Vec<NameSeg>> {
    match_bytes(&[0x2E]).then(
        pair(name_seg(), name_seg())
            .map(|(a, b)| vec![a, b])
            .arced(),
    )
}

fn name_seg<'a>() -> impl Parser<'a, NameSeg> {
    move |input: &'a [u8]| match input.get(0..4) {
        Some(name)
            if ((b'A'..=b'Z').contains(&name[0]) || name[0] == b'_')
                && name[1..4].iter().all(|e| {
                    (b'A'..=b'Z').contains(e) | (b'0'..=b'9').contains(e) | (*e == b'_')
                }) =>
        {
            Ok((
                &input[4..],
                <[u8; 4]>::try_from(name).unwrap().map(|e| e as char),
            ))
        }
        _ => Err(input),
    }
}

fn root_char<'a>() -> impl Parser<'a, ()> {
    match_bytes(&[0x5C])
}

#[test_case]
fn relative_name_string_test() {
    assert_eq!(
        name_string().parse(&[b'_', b'A', b'B', b'_']),
        Ok((
            vec![].as_slice(),
            NameString::Relative(0, vec![['_', 'A', 'B', '_']])
        ))
    );
    assert_eq!(
        name_string().parse(&[b'^', b'E', b'A', b'B', b'_']),
        Ok((
            vec![].as_slice(),
            NameString::Relative(1, vec![['E', 'A', 'B', '_']])
        ))
    );
    assert_eq!(
        name_string()
            .parse(&[b'^', b'^', b'^', 0x2E, b'F', b'A', b'B', b'G', b'_', b'S', b'B', b'_']),
        Ok((
            vec![].as_slice(),
            NameString::Relative(3, vec![['F', 'A', 'B', 'G'], ['_', 'S', 'B', '_']])
        ))
    );
}

#[test_case]
fn absolute_name_string_test() {
    assert_eq!(
        name_string().parse(&[0x5C, b'_', b'S', b'B', b'_']),
        Ok((
            vec![].as_slice(),
            NameString::Absolute(vec![['_', 'S', 'B', '_']])
        ))
    );
}

#[test_case]
fn null_name_path_test() {
    assert_eq!(name_path().parse(&[0x00]), Ok((vec![].as_slice(), None,)));
}

#[test_case]
fn dual_name_path_test() {
    assert_eq!(
        name_path().parse(&[0x2E, b'E', b'A', b'D', b'E', b'E', b'4', b'3', b'2']),
        Ok((
            vec![].as_slice(),
            Some(vec![['E', 'A', 'D', 'E'], ['E', '4', '3', '2']]),
        ))
    );
    assert_eq!(
        name_path().parse(&[0x2E, b'E', b'A', b'D', b'E']),
        Err(vec![0x2E, b'E', b'A', b'D', b'E'].as_slice())
    );
}

#[test_case]
fn multi_name_path_test() {
    let count = 4;
    let mut multiname_test = vec![0x2F, count];
    for _ in 0..(count - 1) {
        multiname_test.extend_from_slice(&[b'A', b'E', b'D', b'G']);
    }
    multiname_test.extend_from_slice(&[b'I', b'F', b'F', b'E']);
    assert_eq!(
        name_path().parse(&multiname_test),
        Ok((
            vec![].as_slice(),
            Some(vec![
                ['A', 'E', 'D', 'G'],
                ['A', 'E', 'D', 'G'],
                ['A', 'E', 'D', 'G'],
                ['I', 'F', 'F', 'E']
            ]),
        ))
    );

    assert_eq!(name_path().parse(&[0x2F, 3]), Err(vec![0x2F, 3].as_slice()));
}

#[test_case]
fn name_path_test() {
    assert_eq!(
        name_path().parse(&[b'_', b'A', b'D', b'E']),
        Ok((vec![].as_slice(), Some(vec![['_', 'A', 'D', 'E']])))
    );
    assert_eq!(
        name_path().parse(&[b'_', b'A', b'2', b'1']),
        Ok((vec![].as_slice(), Some(vec![['_', 'A', '2', '1']])))
    );
    assert_eq!(
        name_path().parse(&[b'e', b'A', b'D', b'E']),
        Err(vec![b'e', b'A', b'D', b'E'].as_slice()),
    );
    assert_eq!(
        name_path().parse(&[b'1', b'A', b'D', b'E']),
        Err(vec![b'1', b'A', b'D', b'E'].as_slice()),
    );
}
