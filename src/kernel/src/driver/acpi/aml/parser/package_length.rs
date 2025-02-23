use bit_field::BitField;

use crate::driver::acpi::aml::AmlContext;

use super::{byte_data, Parser};

pub fn package_length<'a, 'c>() -> impl Parser<'a, 'c, u32>
where
    'c: 'a,
{
    byte_data.and_then(|e| {
        move |mut input: &'a [u8], mut context: &'c mut AmlContext| {
            let msb = e.get_bits(6..=7);
            if msb != 0 {
                let mut result = e.get_bits(0..4) as u32;
                for i in 0..msb as usize {
                    match byte_data.parse(input, context) {
                        Ok((next_input, next_context, byte)) => {
                            result.set_bits((i * 8 + 4)..(i * 8 + 12), byte.into());
                            input = next_input;
                            context = next_context;
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok((input, context, result))
            } else {
                Ok((input, context, e as u32))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::driver::acpi::aml::parser::parser_ok;

    use super::*;

    #[test_case]
    fn package_length_test() {
        let mut context = AmlContext::test_context();
        parser_ok!(package_length(), [0b00011101], &mut context, 0b11101);
        parser_ok!(
            package_length(),
            [0b01001101, 0b10000000],
            &mut context,
            0b100000001101
        );
        parser_ok!(
            package_length(),
            [0b01001101, 0b11111111],
            &mut context,
            0b111111111101
        );
        parser_ok!(
            package_length(),
            [0b10001101, 0b11001101, 0b10101010],
            &mut context,
            0b10101010110011011101
        );
        parser_ok!(
            package_length(),
            [0b11001101, 0b11001101, 0b10101010, 0b01101011],
            &mut context,
            0b0110101110101010110011011101
        );
        // TODO: it should not consume the length when error
        //assert_eq!(
        //    package_length().parse(&[0b11001101, 0b11001101]),
        //    Err(vec![0b11001101, 0b11001101].as_slice())
        //);
    }
}
