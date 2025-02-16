use alloc::{boxed::Box, sync::Arc, vec::Vec};

mod name_string;
mod opcode;
mod package_length;
mod term_object;

#[macro_export]
macro_rules! choose {
    ($first:expr, $($rest:expr),+ $(,)?) => {{
        let next = $first;
        $(
            let next = $crate::driver::acpi::aml::parser::either(next, $rest);
        )+
        next
    }};
}

#[macro_export]
macro_rules! parser_ok {
    ($parser:expr, $input:expr, $expected:expr $(,)?) => {
        assert_eq!(
            $parser.parse(&$input),
            Ok((alloc::vec![].as_slice(), $expected))
        )
    };
}

#[macro_export]
macro_rules! parser_err {
    ($parser:expr, $input:expr, $err:tt $(,)?) => {
        assert_eq!(
            $parser.parse(&$input),
            Err(alloc::vec!$err.as_slice())
        )
    };
    ($parser:expr, $input:tt $(,)?) => {
        assert_eq!(
            $parser.parse(&$input),
            Err(alloc::vec!$input.as_slice())
        )
    };
}

pub type ParserError<'a> = &'a [u8];

type ParseResult<'a, Output> = Result<(&'a [u8], Output), ParserError<'a>>;

pub trait Parser<'a, Output> {
    fn parse(&self, input: &'a [u8]) -> ParseResult<'a, Output>;

    fn map<F, NewOutput>(self, map_fn: F) -> BoxedParser<'a, NewOutput>
    where
        Self: Sized + 'a,
        Output: 'a,
        NewOutput: 'a,
        F: Fn(Output) -> NewOutput + 'a,
    {
        BoxedParser::new(map(self, map_fn))
    }

    fn and_then<F, NextParser, NewOutput>(self, f: F) -> BoxedParser<'a, NewOutput>
    where
        Self: Sized + 'a,
        Output: 'a,
        NewOutput: 'a,
        NextParser: Parser<'a, NewOutput> + 'a,
        F: Fn(Output) -> NextParser + 'a,
    {
        BoxedParser::new(and_then(self, f))
    }

    fn then<P, NewOutput>(self, parser: P) -> BoxedParser<'a, NewOutput>
    where
        Self: Sized + 'a,
        Output: 'a,
        P: Parser<'a, NewOutput> + 'a + Clone,
        NewOutput: 'a,
    {
        BoxedParser::new(and_then(self, move |_| parser.clone()))
    }

    fn arced(self) -> ArcedParser<'a, Output>
    where
        Self: Sized + 'a,
    {
        ArcedParser::new(self)
    }
}

pub struct ArcedParser<'a, Output> {
    parser: Arc<dyn Parser<'a, Output> + 'a>,
}

impl<'a, Output> ArcedParser<'a, Output> {
    pub fn new<P>(parser: P) -> Self
    where
        P: Parser<'a, Output> + 'a,
    {
        ArcedParser {
            parser: Arc::new(parser),
        }
    }
}

impl<'a, Output> Clone for ArcedParser<'a, Output> {
    fn clone(&self) -> Self {
        Self {
            parser: self.parser.clone(),
        }
    }
}

impl<'a, Output> Parser<'a, Output> for ArcedParser<'a, Output> {
    fn parse(&self, input: &'a [u8]) -> ParseResult<'a, Output> {
        self.parser.parse(input)
    }
}

pub struct BoxedParser<'a, Output> {
    parser: Box<dyn Parser<'a, Output> + 'a>,
}

impl<'a, Output> BoxedParser<'a, Output> {
    fn new<P>(parser: P) -> Self
    where
        P: Parser<'a, Output> + 'a,
    {
        BoxedParser {
            parser: Box::new(parser),
        }
    }
}

impl<'a, Output> Parser<'a, Output> for BoxedParser<'a, Output> {
    fn parse(&self, input: &'a [u8]) -> ParseResult<'a, Output> {
        self.parser.parse(input)
    }
}

impl<'a, F, Output> Parser<'a, Output> for F
where
    F: Fn(&'a [u8]) -> ParseResult<'a, Output>,
{
    fn parse(&self, input: &'a [u8]) -> ParseResult<'a, Output> {
        self(input)
    }
}

fn match_bytes<'a>(expected: &'static [u8]) -> impl Parser<'a, ()> {
    move |input: &'a [u8]| match input.get(0..expected.len()) {
        Some(next) if next == expected => Ok((&input[expected.len()..], ())),
        _ => Err(input),
    }
}

fn pair<'a, P1, P2, R1, R2>(parser1: P1, parser2: P2) -> impl Parser<'a, (R1, R2)>
where
    P1: Parser<'a, R1>,
    P2: Parser<'a, R2>,
{
    move |input| {
        parser1.parse(input).and_then(|(next_input, result1)| {
            parser2
                .parse(next_input)
                .map(|(last_input, result2)| (last_input, (result1, result2)))
        })
    }
}

fn either<'a, P1, P2, A>(parser1: P1, parser2: P2) -> impl Parser<'a, A>
where
    P1: Parser<'a, A>,
    P2: Parser<'a, A>,
{
    move |input| match parser1.parse(input) {
        ok @ Ok(_) => ok,
        Err(_) => parser2.parse(input),
    }
}

fn map<'a, P, F, A, B>(parser: P, map_fn: F) -> impl Parser<'a, B>
where
    P: Parser<'a, A>,
    F: Fn(A) -> B,
{
    move |input| {
        parser
            .parse(input)
            .map(|(next_input, result)| (next_input, map_fn(result)))
    }
}

fn left<'a, P1, P2, R1, R2>(parser1: P1, parser2: P2) -> impl Parser<'a, R1>
where
    P1: Parser<'a, R1>,
    P2: Parser<'a, R2>,
{
    map(pair(parser1, parser2), |(left, _right)| left)
}

fn right<'a, P1, P2, R1, R2>(parser1: P1, parser2: P2) -> impl Parser<'a, R2>
where
    P1: Parser<'a, R1>,
    P2: Parser<'a, R2>,
{
    map(pair(parser1, parser2), |(_left, right)| right)
}

fn and_then<'a, P, F, A, B, NextP>(parser: P, f: F) -> impl Parser<'a, B>
where
    P: Parser<'a, A>,
    NextP: Parser<'a, B>,
    F: Fn(A) -> NextP,
{
    move |input| match parser.parse(input) {
        Ok((next_input, result)) => f(result).parse(next_input),
        Err(err) => Err(err),
    }
}

fn zero_or_more<'a, P, A>(parser: P) -> impl Parser<'a, Vec<A>>
where
    P: Parser<'a, A>,
{
    move |mut input| {
        let mut result = Vec::new();

        while let Ok((next_input, next_item)) = parser.parse(input) {
            input = next_input;
            result.push(next_item);
        }

        Ok((input, result))
    }
}

fn byte_data(input: &[u8]) -> ParseResult<u8> {
    match input.iter().next() {
        Some(next) => Ok((&input[1..], *next)),
        _ => Err(input),
    }
}
