use bit_field::BitField;

use super::{byte_data, Parser};
use alloc::vec;

fn package_length<'a>() -> impl Parser<'a, u32> {
    byte_data.and_then(|e| {
        move |mut input: &'a [u8]| {
            let msb = e.get_bits(6..=7);
            if msb != 0 {
                let mut result = e.get_bits(0..4) as u32;
                for i in 0..msb as usize {
                    match byte_data.parse(input) {
                        Ok((next_input, byte)) => {
                            result.set_bits((i * 8 + 4)..(i * 8 + 12), byte.into());
                            input = next_input;
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok((input, result))
            } else {
                Ok((input, e as u32))
            }
        }
    })
}

#[test_case]
fn package_length_test() {
    assert_eq!(
        package_length().parse(&[0b00011101]),
        Ok((vec![].as_slice(), 0b11101))
    );
    assert_eq!(
        package_length().parse(&[0b01001101, 0b10000000]),
        Ok((vec![].as_slice(), 0b100000001101))
    );
    assert_eq!(
        package_length().parse(&[0b01001101, 0b11111111]),
        Ok((vec![].as_slice(), 0b111111111101))
    );
    assert_eq!(
        package_length().parse(&[0b10001101, 0b11001101, 0b10101010]),
        Ok((vec![].as_slice(), 0b10101010110011011101))
    );
    assert_eq!(
        package_length().parse(&[0b11001101, 0b11001101, 0b10101010, 0b01101011]),
        Ok((vec![].as_slice(), 0b0110101110101010110011011101))
    );
    // TODO: it should not consume the length when error
    //assert_eq!(
    //    package_length().parse(&[0b11001101, 0b11001101]),
    //    Err(vec![0b11001101, 0b11001101].as_slice())
    //);
}
