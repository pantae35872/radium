use core::{iter, ops::ControlFlow};
use named_obj::named_obj;
use namespace_modifier::namespace_modifier;

use crate::{
    driver::acpi::aml::{parser::choose, AmlContext, AmlError, AmlValue},
    log,
};

use super::{Parser, Propagate};

mod named_obj;
mod namespace_modifier;

pub fn term_object<'a, 'c>() -> impl Parser<'a, 'c, Option<AmlValue>>
where
    'c: 'a,
{
    choose!(
        namespace_modifier().map(|_| None),
        named_obj().map(|()| None),
        //            statement_opcode().map(|()| Ok(None)),
        //            expression_opcode().map(|value| Ok(Some(value)))
    )
}

fn term_list<'a, 'c>(list_length: u32) -> impl Parser<'a, 'c, ()>
where
    'c: 'a,
{
    move |o_input: &'a [u8], context: &'c mut AmlContext| match iter::repeat(()).try_fold(
        (list_length, o_input, context),
        |(list_length, input, context), _| {
            if list_length == 0 {
                return ControlFlow::Break((list_length, input, context, None));
            }
            match term_object().parse(input, context) {
                Ok((next_input, next_context, None))
                    if list_length
                        .checked_sub((o_input.len() - next_input.len()) as u32)
                        .is_some() =>
                {
                    ControlFlow::Continue((
                        list_length - (o_input.len() - next_input.len()) as u32,
                        next_input,
                        next_context,
                    ))
                }
                Ok((next_input, next_context, None)) => ControlFlow::Break((
                    list_length,
                    next_input,
                    next_context,
                    Some(AmlError::ParserError.into()),
                )),
                Ok((next_input, next_context, Some(value))) => ControlFlow::Break((
                    list_length,
                    next_input,
                    next_context,
                    Some(Propagate::Return(value)),
                )),
                Err((next_input, next_context, propagate)) => {
                    ControlFlow::Break((list_length, next_input, next_context, Some(propagate)))
                }
            }
        },
    ) {
        ControlFlow::Continue(..) => {
            unreachable!("the iterator is repeate(()) which never ends until break")
        }
        ControlFlow::Break((length, next_input, next_context, None)) => {
            assert_eq!(length, 0);
            Ok((next_input, next_context, ()))
        }
        ControlFlow::Break((length, next_input, next_context, Some(propagate))) => {
            log!(Trace, "Propagating with left over length: {length}");
            Err((next_input, next_context, propagate))
        }
    }
}
