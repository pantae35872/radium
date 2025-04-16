use core::ops::ControlFlow;

use alloc::{boxed::Box, sync::Arc, vec::Vec};

use super::{AmlContext, AmlError, AmlValue};

mod name_string;
mod opcode;
mod package_length;
pub mod term_object;

macro choose($first:expr, $($rest:expr),+ $(,)?) {{
    let next = $first;
    $(
        let next = $crate::driver::acpi::aml::parser::either(next, $rest);
    )+
    next
}}
macro_rules! parser_ok {
    ($parser:expr, $input:expr, $context:expr, $expected:expr $(,)?) => {
        assert_eq!(
            $parser
                .parse(&$input, $context)
                .map(|(l, _, r)| (l, r))
                .map_err(|(e, _, r)| (e, r)),
            Ok((alloc::vec![].as_slice(), $expected))
        )
    };
    ($parser:expr, $input:expr, $context:expr $(,)?) => {
        assert_eq!(
            $parser
                .parse(&$input, $context)
                .map(|(l, _, r)| (l, r))
                .map_err(|(e, _, r)| (e, r)),
            Ok((alloc::vec![].as_slice(), ()))
        )
    };
}

macro_rules! parser_err {
    ($parser:expr, $input:expr, $context:expr, $aml_err:expr $(,)?) => {
        assert_eq!(
            $parser.parse(&$input, $context)
                .map(|(l, _, r)| (l, r))
                .map_err(|(e, _, r)| (e, r)),
            Err((alloc::vec!$input.as_slice(), $aml_err))
        )
    };

    ($parser:expr, $input:expr, $context:expr, $err:tt $(,)?) => {
        assert_eq!(
            $parser.parse(&$input, $context)
                .map(|(l, _, r)| (l, r))
                .map_err(|(e, _, r)| (e, r)),
            Err(alloc::vec!$err.as_slice())
        )
    };

    ($parser:expr, $input:tt, $context:expr $(,)?) => {
        assert_eq!(
            $parser.parse(&$input, $context)
                .map(|(l, _, r)| (l, r))
                .map_err(|(e, _, r)| (e, r)),
            Err((alloc::vec!$input.as_slice(), AmlError::ParserError.into()))
        )
    };
}


macro try_with_context($context: expr, $expr: expr) {
    match $expr {
        Ok(result) => result,
        Err(err) => return (Err(err.into()), $context),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Propagate {
    AmlError(AmlError),
    Return(AmlValue),
}

pub type ParserError<'a, 'c> = (&'a [u8], &'c mut AmlContext, Propagate);

type ParseResult<'a, 'c, Output> =
    Result<(&'a [u8], &'c mut AmlContext, Output), ParserError<'a, 'c>>;

pub trait Parser<'a, 'c, Output>
where
    'c: 'a,
{
    fn parse(&self, input: &'a [u8], context: &'c mut AmlContext) -> ParseResult<'a, 'c, Output>;

    fn map<F, NewOutput>(self, map_fn: F) -> BoxedParser<'a, 'c, NewOutput>
    where
        Self: Sized + 'a,
        Output: 'a,
        NewOutput: 'a,
        F: Fn(Output) -> NewOutput + 'a,
    {
        BoxedParser::new(map(self, map_fn))
    }

    fn map_with_context<F, NewOutput>(self, map_fn: F) -> BoxedParser<'a, 'c, NewOutput>
    where
        Self: Sized + 'a,
        NewOutput: 'a,
        Output: 'a,
        F: Fn(Output, &'c mut AmlContext) -> (Result<NewOutput, Propagate>, &'c mut AmlContext)
            + 'a,
    {
        BoxedParser::new(map_with_context(self, map_fn))
    }

    fn and_then<F, NextParser, NewOutput>(self, f: F) -> BoxedParser<'a, 'c, NewOutput>
    where
        Self: Sized + 'a,
        Output: 'a,
        NewOutput: 'a,
        NextParser: Parser<'a, 'c, NewOutput> + 'a,
        F: Fn(Output) -> NextParser + 'a,
    {
        BoxedParser::new(and_then(self, f))
    }

    fn then<P, NewOutput>(self, parser: P) -> BoxedParser<'a, 'c, NewOutput>
    where
        Self: Sized + 'a,
        Output: 'a,
        P: Parser<'a, 'c, NewOutput> + 'a + Clone,
        NewOutput: 'a,
    {
        BoxedParser::new(and_then(self, move |_| parser.clone()))
    }

    fn arced(self) -> ArcedParser<'a, 'c, Output>
    where
        Self: Sized + 'a,
    {
        ArcedParser::new(self)
    }
}

pub struct ArcedParser<'a, 'c, Output> {
    parser: Arc<dyn Parser<'a, 'c, Output> + 'a>,
}

impl<'a, 'c, Output> ArcedParser<'a, 'c, Output> {
    pub fn new<P>(parser: P) -> Self
    where
        P: Parser<'a, 'c, Output> + 'a,
    {
        ArcedParser {
            parser: Arc::new(parser),
        }
    }
}

impl<'a, 'c, Output> Clone for ArcedParser<'a, 'c, Output> {
    fn clone(&self) -> Self {
        Self {
            parser: self.parser.clone(),
        }
    }
}

impl<'a, 'c, Output> Parser<'a, 'c, Output> for ArcedParser<'a, 'c, Output> {
    fn parse(&self, input: &'a [u8], context: &'c mut AmlContext) -> ParseResult<'a, 'c, Output> {
        self.parser.parse(input, context)
    }
}

pub struct BoxedParser<'a, 'c, Output> {
    parser: Box<dyn Parser<'a, 'c, Output> + 'a>,
}

impl<'a, 'c, Output> BoxedParser<'a, 'c, Output> {
    fn new<P>(parser: P) -> Self
    where
        P: Parser<'a, 'c, Output> + 'a,
        'c: 'a,
    {
        BoxedParser {
            parser: Box::new(parser),
        }
    }
}

impl<'a, 'c, Output> Parser<'a, 'c, Output> for BoxedParser<'a, 'c, Output> {
    fn parse(&self, input: &'a [u8], context: &'c mut AmlContext) -> ParseResult<'a, 'c, Output> {
        self.parser.parse(input, context)
    }
}

impl<'a, 'c, F, Output> Parser<'a, 'c, Output> for F
where
    F: Fn(&'a [u8], &'c mut AmlContext) -> ParseResult<'a, 'c, Output>,
    'c: 'a,
{
    fn parse(&self, input: &'a [u8], context: &'c mut AmlContext) -> ParseResult<'a, 'c, Output> {
        self(input, context)
    }
}

fn match_bytes<'a, 'c>(expected: &'static [u8]) -> impl Parser<'a, 'c, ()>
where
    'c: 'a,
{
    move |input: &'a [u8], context| match input.get(0..expected.len()) {
        Some(next) if next == expected => Ok((&input[expected.len()..], context, ())),
        _ => Err((input, context, AmlError::ParserError.into())),
    }
}

fn pair<'a, 'c, P1, P2, R1, R2>(parser1: P1, parser2: P2) -> impl Parser<'a, 'c, (R1, R2)>
where
    'c: 'a,
    P1: Parser<'a, 'c, R1>,
    P2: Parser<'a, 'c, R2>,
{
    move |input, context| {
        parser1
            .parse(input, context)
            .and_then(|(next_input, next_context, result1)| {
                parser2.parse(next_input, next_context).map(
                    |(last_input, last_context, result2)| {
                        (last_input, last_context, (result1, result2))
                    },
                )
            })
    }
}

fn either<'a, 'c, P1, P2, A>(parser1: P1, parser2: P2) -> impl Parser<'a, 'c, A>
where
    'c: 'a,
    P1: Parser<'a, 'c, A>,
    P2: Parser<'a, 'c, A>,
{
    move |input: &'a [u8], context: &'c mut AmlContext| match parser1.parse(input, context) {
        ok @ Ok(_) => ok,
        Err((_, context, _)) => parser2.parse(input, context),
    }
}

fn map_with_context<'a, 'c, P, F, A, B>(parser: P, map_fn: F) -> impl Parser<'a, 'c, B>
where
    P: Parser<'a, 'c, A>,
    F: Fn(A, &'c mut AmlContext) -> (Result<B, Propagate>, &'c mut AmlContext),
    'c: 'a,
{
    move |input, context| match parser.parse(input, context) {
        Ok((next_input, next_context, result)) => match map_fn(result, next_context) {
            (Ok(result_value), context) => Ok((next_input, context, result_value)),
            (Err(err), context) => Err((input, context, err)),
        },
        Err(result) => Err(result),
    }
}

fn map<'a, 'c, P, F, A, B>(parser: P, map_fn: F) -> impl Parser<'a, 'c, B>
where
    P: Parser<'a, 'c, A>,
    F: Fn(A) -> B,
    'c: 'a,
{
    move |input, context| {
        parser
            .parse(input, context)
            .map(|(next_input, context, result)| (next_input, context, map_fn(result)))
    }
}

fn left<'a, 'c, P1, P2, R1, R2>(parser1: P1, parser2: P2) -> impl Parser<'a, 'c, R1>
where
    'c: 'a,
    P1: Parser<'a, 'c, R1>,
    P2: Parser<'a, 'c, R2>,
{
    map(pair(parser1, parser2), |(left, _right)| left)
}

fn right<'a, 'c, P1, P2, R1, R2>(parser1: P1, parser2: P2) -> impl Parser<'a, 'c, R2>
where
    'c: 'a,
    P1: Parser<'a, 'c, R1>,
    P2: Parser<'a, 'c, R2>,
{
    map(pair(parser1, parser2), |(_left, right)| right)
}

fn and_then<'a, 'c, P, F, A, B, NextP>(parser: P, f: F) -> impl Parser<'a, 'c, B>
where
    'c: 'a,
    P: Parser<'a, 'c, A>,
    NextP: Parser<'a, 'c, B>,
    F: Fn(A) -> NextP,
{
    move |input, context| match parser.parse(input, context) {
        Ok((next_input, context, result)) => f(result).parse(next_input, context),
        Err(err) => Err(err),
    }
}

fn zero_or_more<'a, 'c, P, A>(parser: P) -> impl Parser<'a, 'c, Vec<A>>
where
    'c: 'a,
    P: Parser<'a, 'c, A>,
{
    move |input: &'a [u8], context: &'c mut AmlContext| match core::iter::repeat(()).try_fold(
        (input, context, Vec::new()),
        |(input, context, mut result), _| match parser.parse(input, context) {
            Ok((next_input, next_context, item)) => {
                result.push(item);
                ControlFlow::Continue((next_input, next_context, result))
            }
            Err((_next_input, next_context, _)) => {
                ControlFlow::Break((input, next_context, result))
            }
        },
    ) {
        ControlFlow::Continue(result) | ControlFlow::Break(result) => return Ok(result),
    }
}

fn byte_data<'a, 'c>(input: &'a [u8], context: &'c mut AmlContext) -> ParseResult<'a, 'c, u8> {
    match input.iter().next() {
        Some(next) => Ok((&input[1..], context, *next)),
        _ => Err((input, context, AmlError::ParserError.into())),
    }
}
