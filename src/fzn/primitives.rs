//! Parsers for tokens used throughout the FlatZinc grammar.

use std::fmt::{Debug, Display};

use rangelist::RangeList;
use winnow::{
	Parser, Result,
	ascii::{digit1, hex_digit1, multispace1, oct_digit1},
	combinator::{alt, delimited, opt, separated, separated_pair, trace},
	error::{ContextError, FromExternalError},
	stream::AsChar,
	token::{one_of, take_till, take_until, take_while},
};

use crate::{Literal, fzn::Stream};

/// Parse a `/* ... */` block comment.
fn block_comment<I, F>(input: &mut Stream<'_, '_, I, F>) -> Result<()>
where
	I: Debug,
{
	delimited("/*", take_until(0.., "*/"), "*/")
		.void()
		.parse_next(input)
}

/// Parses a boolean literal.
///
/// ```bnf
/// <bool-literal> ::= "false"
///                  | "true"
/// ```
pub(super) fn boolean<I: Debug, F>(input: &mut Stream<'_, '_, I, F>) -> Result<bool> {
	alt(("true".map(|_| true), "false".map(|_| false))).parse_next(input)
}

/// Parses a list of elements seperated by a comma, and delimited by
/// `open_token` and `close_token`.
pub(super) fn delimited_list<'source, 'state, T, I, F: 'state>(
	open_token: &'static str,
	element_parser: impl Parser<Stream<'source, 'state, I, F>, T, ContextError>,
	close_token: &'static str,
) -> impl Parser<Stream<'source, 'state, I, F>, Vec<T>, ContextError>
where
	I: Debug + 'state,
{
	delimited(
		token(open_token),
		separated(0.., token(element_parser), token(",")),
		token(close_token),
	)
}

/// Parses a float literal from the input.
///
/// ```bnf
/// <float-literal> ::= [-]?[0-9]+.[0-9]+
///                   | [-]?[0-9]+.[0-9]+[Ee][-+]?[0-9]+
///                   | [-]?[0-9]+[Ee][-+]?[0-9]+
/// ```
pub(super) fn float<I: Debug, F>(input: &mut Stream<'_, '_, I, F>) -> Result<f64> {
	trace("float", move |input: &mut Stream<'_, '_, I, F>| {
		(
			opt('-'),
			digit1,
			alt((
				(
					'.',
					digit1,
					one_of(['e', 'E']),
					opt(one_of(['-', '+'])),
					digit1,
				)
					.take(),
				(one_of(['e', 'E']), opt(one_of(['-', '+'])), digit1).take(),
				('.', digit1).take(),
			)),
		)
			.take()
			.try_map(|parsed: &str| parsed.parse::<f64>())
			.parse_next(input)
	})
	.parse_next(input)
}

/// Parses an identifier.
///
/// ```bnf
/// <var-par-identifier> ::= [A-Za-z_][A-Za-z0-9_]*
/// ```
pub(super) fn identifier<I, F, E>(input: &mut Stream<'_, '_, I, F>) -> Result<I>
where
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
	I: Debug,
{
	let ident = identifier_raw.parse_next(input)?;
	input
		.state
		.intern(ident)
		.map_err(|err| ContextError::from_external_error(input, err))
}

/// Parses an identifier.
///
/// ```bnf
/// <var-par-identifier> ::= [A-Za-z_][A-Za-z0-9_]*
/// ```
pub(super) fn identifier_raw<'a, I, F>(input: &mut Stream<'a, '_, I, F>) -> Result<&'a str>
where
	I: Debug,
{
	trace(
		"identifier",
		(
			one_of(|c: char| c.is_alpha() || c == '_'),
			take_while(0.., |c: char| c.is_alphanum() || c == '_'),
		),
	)
	.take()
	.parse_next(input)
}

/// Parse insignificant whitespace and comments.
fn ignored<I, F>(input: &mut Stream<'_, '_, I, F>) -> Result<()>
where
	I: Debug,
{
	while alt((
		multispace1.void(),
		line_comment.void(),
		block_comment.void(),
	))
	.parse_next(input)
	.is_ok()
	{}

	Ok(())
}

/// Parses an integer literal from the input.
///
/// ```bnf
/// <int-literal> ::= [-]?[0-9]+
///                 | [-]?0x[0-9A-Fa-f]+
///                 | [-]?0o[0-7]+
/// ```
pub(super) fn int<I: Debug, F>(input: &mut Stream<'_, '_, I, F>) -> Result<i64> {
	trace("int", move |input: &mut Stream<'_, '_, I, F>| {
		let is_negative = opt('-').parse_next(input)?.is_some();

		let unsigned_integer = alt((
			("0x", hex_digit1).try_map(|(_, hex)| i64::from_str_radix(hex, 16)),
			("0o", oct_digit1).try_map(|(_, octal)| i64::from_str_radix(octal, 8)),
			digit1.try_map(|base_ten: &str| base_ten.parse::<i64>()),
		))
		.parse_next(input)?;

		if is_negative {
			Ok(-unsigned_integer)
		} else {
			Ok(unsigned_integer)
		}
	})
	.parse_next(input)
}

/// Higher-order parser for `<token> .. <token>`.
pub(super) fn interval_set<'source, 'state, T, I, F: 'state>(
	elem_parser: impl Parser<Stream<'source, 'state, I, F>, T, ContextError> + Copy,
) -> impl Parser<Stream<'source, 'state, I, F>, RangeList<T>, ContextError>
where
	T: PartialOrd + Copy + 'static,
	I: Debug + 'state,
{
	move |input: &mut Stream<'source, 'state, I, F>| {
		separated_pair(token(elem_parser), token(".."), token(elem_parser))
			.map(|(start, end)| RangeList::from_iter([start..=end]))
			.parse_next(input)
	}
}

/// Parse a `%` line comment.
fn line_comment<I, F>(input: &mut Stream<'_, '_, I, F>) -> Result<()>
where
	I: Debug,
{
	('%', take_till(0.., |c| c == '\n'), opt('\n'))
		.void()
		.parse_next(input)
}

/// Parses a basic literal expression.
///
/// ```bnf
/// <basic-literal-expr> ::= <bool-literal>
///                        | <int-literal>
///                        | <float-literal>
///                        | <set-literal>
/// ```
pub(super) fn literal<'a, 's, I, F, E>(input: &mut Stream<'a, 's, I, F>) -> Result<Literal<I>>
where
	I: Clone + Debug,
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
{
	// This can be optimized if it turns out to be a bottleneck. At the moment, to
	// parse a literal, it will first attempt to parse a float and, if that fails,
	// parse an integer. We can be more clever about that by peeking at the next
	// character to determine what is being parsed.

	let parsed_literal = alt((
		set(int).map(Literal::IntSet),
		set(float).map(Literal::FloatSet),
		boolean.map(Literal::Bool),
		float.map(Literal::Float),
		int.map(Literal::Int),
		identifier_raw.map(Literal::Identifier),
	))
	.parse_next(input)?;

	Ok(match parsed_literal {
		Literal::Identifier(ident) => {
			if let Some(literal) = input.state.resolve(ident).cloned() {
				literal
			} else {
				Literal::Identifier(
					input
						.state
						.intern(ident)
						.map_err(|err| ContextError::from_external_error(input, err))?,
				)
			}
		}
		Literal::Int(i) => Literal::Int(i),
		Literal::Float(f) => Literal::Float(f),
		Literal::Bool(b) => Literal::Bool(b),
		Literal::IntSet(r) => Literal::IntSet(r),
		Literal::FloatSet(r) => Literal::FloatSet(r),
		Literal::String(s) => Literal::String(s),
	})
}

/// Parses a set literal.
///
/// Works with either interval sets or sparse sets.
///
/// The grammar is modified from the documentation. Here we abstract the element
/// type.
///
/// ```bnf
/// <set-literal> ::= <set-term> [ "union" <set-term> ] ...
///
/// <set-term> ::= "{" [ <elem> "," ... ] "}"
///              | <elem> ".." <elem>
/// ```
pub(super) fn set<'source, 'state, T, I, F: 'state>(
	elem_parser: impl Parser<Stream<'source, 'state, I, F>, T, ContextError> + Copy,
) -> impl Parser<Stream<'source, 'state, I, F>, RangeList<T>, ContextError>
where
	I: Debug + 'state,
	T: PartialOrd + Copy + 'static,
{
	move |input: &mut Stream<'source, 'state, I, F>| -> Result<RangeList<T>> {
		let sparse_set = delimited(
			token('{'),
			separated(0.., token(elem_parser), token(',')),
			token('}'),
		)
		.map(|elems: Vec<T>| RangeList::from_iter(elems.into_iter().map(|elem| elem..=elem)));

		let set_term = alt((sparse_set, interval_set(elem_parser)));
		let mut set_union = separated(1.., token(set_term), token("union"))
			.map(|ranges: Vec<RangeList<T>>| RangeList::from_iter(ranges.into_iter().flatten()));

		set_union.parse_next(input)
	}
}

/// Parses a token from the input.
///
/// Wraps the given parser with optional preceding and succeeding whitespace or
/// comments.
pub(super) fn token<'source, 'state, T, I, F: 'state>(
	parser: impl Parser<Stream<'source, 'state, I, F>, T, ContextError>,
) -> impl Parser<Stream<'source, 'state, I, F>, T, ContextError>
where
	I: Debug + 'state,
{
	delimited(ignored, parser, ignored)
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use rangelist::RangeList;
	use winnow::{Parser, Stateful};

	use crate::{
		Literal,
		fzn::{ParseState, literal, tests::check_parser},
	};

	#[test]
	fn boolean_literal() {
		check_parser(literal, Literal::Bool(true), "true");
		check_parser(literal, Literal::Bool(false), "false");
	}

	#[test]
	fn float_literal() {
		check_parser(literal, Literal::Float(3.02), "3.02");
		check_parser(literal, Literal::Float(-34.85), "-34.85");
		check_parser(literal, Literal::Float(5e-1), "5e-1");
		check_parser(literal, Literal::Float(5e12), "5e12");
		check_parser(literal, Literal::Float(-11e3), "-11e3");
		check_parser(literal, Literal::Float(5e-1), "5E-1");
		check_parser(literal, Literal::Float(5e12), "5E12");
		check_parser(literal, Literal::Float(-11e3), "-11E3");
		check_parser(literal, Literal::Float(5.2e-1), "5.2E-1");
		check_parser(literal, Literal::Float(5.54e12), "5.54E12");
		check_parser(literal, Literal::Float(-11e3), "-11E+3");
	}

	#[test]
	fn float_set_literal() {
		check_parser(literal, Literal::IntSet(RangeList::from(1..=5)), "1..5");
		check_parser(
			literal,
			Literal::FloatSet(RangeList::from_iter([1.3..=1.3, 4e3..=4e3, -4.8..=-4.8])),
			"{1.3, 4e3, -4.8}",
		);
		check_parser(
			literal,
			Literal::FloatSet(RangeList::from_iter([2.0..=2.0, 2.5..=3.0])),
			"2.0..2.0 union 2.5..3.0",
		);
		check_parser(
			literal,
			Literal::FloatSet(RangeList::from_iter([1.0..=1.0, 2.5..=3.0])),
			"{1.0} union 2.5..3.0",
		);
	}

	#[test]
	fn identifier_literal() {
		check_parser(
			literal,
			Literal::Identifier("some_name".to_owned()),
			"some_name",
		);
		check_parser(
			literal,
			Literal::Identifier("_some_name".to_owned()),
			"_some_name",
		);
		check_parser(
			literal,
			Literal::Identifier("_SomeName283".to_owned()),
			"_SomeName283",
		);
	}

	#[test]
	fn identifiers_of_parameters_are_resolved() {
		let mut aliases =
			HashMap::from_iter([("some_param".to_owned(), Literal::<String>::Int(5))]);
		let mut interner = |s: &str| Ok::<_, std::convert::Infallible>(s.to_owned());

		let stream = Stateful {
			input: "some_param",
			state: ParseState::new(&mut aliases, &mut interner),
		};

		let parsed = literal.parse(stream);
		match parsed {
			Ok(actual) => assert_eq!(Literal::Int(5), actual),
			Err(err) => panic!("expected parameter alias to resolve: {err:?}"),
		}
	}

	#[test]
	fn int_literal() {
		check_parser(literal, Literal::Int(0), "0");
		check_parser(literal, Literal::Int(420), "420");
		check_parser(literal, Literal::Int(-38), "-38");
		check_parser(literal, Literal::Int(0xff32a), "0xff32a");
		check_parser(literal, Literal::Int(-0xadc20), "-0xadc20");
		check_parser(literal, Literal::Int(0o12356), "0o12356");
		check_parser(literal, Literal::Int(-0o230), "-0o230");
	}

	#[test]
	fn int_set_literal() {
		check_parser(literal, Literal::IntSet(RangeList::from(1..=5)), "1..5");
		check_parser(
			literal,
			Literal::IntSet(RangeList::from_iter([1..=1, 4..=4, 6..=6])),
			"{1, 4, 6}",
		);
		check_parser(
			literal,
			Literal::IntSet(RangeList::from_iter([1..=2, 4..=6])),
			"1..2 union 4..6",
		);
		check_parser(
			literal,
			Literal::IntSet(RangeList::from_iter([1..=1, 4..=5])),
			"{1} union 4..5",
		);
	}
}
