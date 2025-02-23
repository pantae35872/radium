use crate::driver::acpi::aml::parser::Parser;

pub fn named_obj<'a, 'c>() -> impl Parser<'a, 'c, ()>
where
    'c: 'a,
{
    |input, context| Ok((input, context, ()))
}
