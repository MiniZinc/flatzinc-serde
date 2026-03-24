//! Parsers for tokens used throughout the FlatZinc grammar.

use rangelist::RangeList;
use winnow::{
	Parser, Result,
	ascii::{digit1, hex_digit1, multispace1, oct_digit1},
	combinator::{alt, delimited, opt, separated, separated_pair, trace},
	error::ContextError,
	stream::AsChar,
	token::{one_of, take_till, take_until, take_while},
};

use crate::{fzn::Stream, intermediate::Literal};

/// Parse a `/* ... */` block comment.
fn block_comment<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<()> {
	delimited("/*", take_until(0.., "*/"), "*/")
		.void()
		.parse_next(input)
}

/// Parses a boolean literal.
///
/// ```bnf
/// <bool-literal> ::= "true" | "false"
/// ```
pub(super) fn boolean<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<bool> {
	alt(("true".map(|_| true), "false".map(|_| false))).parse_next(input)
}

/// Parses a list of elements separated by a comma and delimited by the given
/// opening and closing tokens.
pub(super) fn delimited_list<'source, Identifier, F, T>(
	open_token: &'static str,
	element_parser: impl Parser<Stream<'source, Identifier, F>, T, ContextError>,
	close_token: &'static str,
) -> impl Parser<Stream<'source, Identifier, F>, Vec<T>, ContextError> {
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
pub(super) fn float<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<f64> {
	trace("float", move |input: &mut Stream<'_, Identifier, F>| {
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

/// Parses an internable identifier as borrowed source text.
///
/// ```bnf
/// <var-par-identifier> ::= [A-Za-z_][A-Za-z0-9_]*
/// ```
pub(super) fn identifier<'a, Identifier, F>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<&'a str> {
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
fn ignored<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<()> {
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
pub(super) fn int<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<i64> {
	trace("int", move |input: &mut Stream<'_, Identifier, F>| {
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
pub(super) fn interval_set<'source, Identifier, F, T>(
	elem_parser: impl Parser<Stream<'source, Identifier, F>, T, ContextError> + Copy,
) -> impl Parser<Stream<'source, Identifier, F>, RangeList<T>, ContextError>
where
	T: PartialOrd + Copy + 'static,
{
	move |input: &mut Stream<'source, Identifier, F>| {
		separated_pair(token(elem_parser), token(".."), token(elem_parser))
			.map(|(start, end)| RangeList::from_iter([start..=end]))
			.parse_next(input)
	}
}

/// Parse a `%` line comment.
fn line_comment<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<()> {
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
///                        | <string-literal>
/// ```
pub(super) fn literal<'a, Identifier, F>(input: &mut Stream<'a, Identifier, F>) -> Result<Literal> {
	enum ParsedLiteral<'a> {
		Literal(Literal),
		Identifier(&'a str),
	}

	// This can be optimized if it turns out to be a bottleneck. At the moment, to
	// parse a literal, it will first attempt to parse a float and, if that fails,
	// parse an integer. We can be more clever about that by peeking at the next
	// character to determine what is being parsed.
	let parsed_literal = alt((
		set(int).map(Literal::IntSet).map(ParsedLiteral::Literal),
		set(float)
			.map(Literal::FloatSet)
			.map(ParsedLiteral::Literal),
		boolean.map(Literal::Bool).map(ParsedLiteral::Literal),
		float.map(Literal::Float).map(ParsedLiteral::Literal),
		int.map(Literal::Int).map(ParsedLiteral::Literal),
		string.map(Literal::String).map(ParsedLiteral::Literal),
		identifier.map(ParsedLiteral::Identifier),
	))
	.parse_next(input)?;

	Ok(match parsed_literal {
		ParsedLiteral::Literal(literal) => literal,
		ParsedLiteral::Identifier(ident) => Literal::Reference(input.state.intern_name(ident)),
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
pub(super) fn set<'source, Identifier, F, T>(
	elem_parser: impl Parser<Stream<'source, Identifier, F>, T, ContextError> + Copy,
) -> impl Parser<Stream<'source, Identifier, F>, RangeList<T>, ContextError>
where
	T: PartialOrd + Copy + 'static,
{
	fn set_literal<'source, Identifier, F, T>(
		elem_parser: impl Parser<Stream<'source, Identifier, F>, T, ContextError> + Copy,
	) -> impl Parser<Stream<'source, Identifier, F>, RangeList<T>, ContextError>
	where
		T: PartialOrd + Copy + 'static,
	{
		move |input: &mut Stream<'source, Identifier, F>| {
			delimited_list("{", elem_parser, "}")
				.map(|values: Vec<T>| values.into_iter().map(|x| x..=x).collect())
				.parse_next(input)
		}
	}

	move |input: &mut Stream<'source, Identifier, F>| {
		separated(
			1..,
			alt((interval_set(elem_parser), set_literal(elem_parser))),
			token("union"),
		)
		.map(|values: Vec<RangeList<T>>| values.into_iter().flatten().collect())
		.parse_next(input)
	}
}

/// Parse a `%` line comment.
fn string<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<String> {
	delimited('"', take_till(0.., |c| c == '"'), '"')
		.map(String::from)
		.parse_next(input)
}

/// Parse optional whitespace around a token parser.
pub(super) fn token<'source, Identifier, F, T>(
	parser: impl Parser<Stream<'source, Identifier, F>, T, ContextError>,
) -> impl Parser<Stream<'source, Identifier, F>, T, ContextError> {
	delimited(ignored, parser, ignored)
}

#[cfg(test)]
mod tests {

	use rangelist::RangeList;

	use crate::{
		fzn::{
			literal,
			tests::{name_id, parse_with_names},
		},
		intermediate::Literal,
	};

	#[test]
	fn boolean_literal() {
		assert_eq!(parse_with_names(literal, "true").0, Literal::Bool(true));
		assert_eq!(parse_with_names(literal, "false").0, Literal::Bool(false));
	}

	#[test]
	fn float_literal() {
		assert_eq!(parse_with_names(literal, "3.02").0, Literal::Float(3.02));
		assert_eq!(
			parse_with_names(literal, "-34.85").0,
			Literal::Float(-34.85)
		);
		assert_eq!(parse_with_names(literal, "5e-1").0, Literal::Float(5e-1));
		assert_eq!(parse_with_names(literal, "5e12").0, Literal::Float(5e12));
		assert_eq!(parse_with_names(literal, "-11e3").0, Literal::Float(-11e3));
		assert_eq!(parse_with_names(literal, "5E-1").0, Literal::Float(5e-1));
		assert_eq!(parse_with_names(literal, "5E12").0, Literal::Float(5e12));
		assert_eq!(parse_with_names(literal, "-11E3").0, Literal::Float(-11e3));
		assert_eq!(
			parse_with_names(literal, "5.2E-1").0,
			Literal::Float(5.2e-1)
		);
		assert_eq!(
			parse_with_names(literal, "5.54E12").0,
			Literal::Float(5.54e12)
		);
		assert_eq!(parse_with_names(literal, "-11E+3").0, Literal::Float(-11e3));
	}

	#[test]
	fn float_set_literal() {
		assert_eq!(
			parse_with_names(literal, "1..5").0,
			Literal::IntSet(RangeList::from(1..=5))
		);
		assert_eq!(
			parse_with_names(literal, "{1.3, 4e3, -4.8}").0,
			Literal::FloatSet(RangeList::from_iter([1.3..=1.3, 4e3..=4e3, -4.8..=-4.8]))
		);
		assert_eq!(
			parse_with_names(literal, "2.0..2.0 union 2.5..3.0").0,
			Literal::FloatSet(RangeList::from_iter([2.0..=2.0, 2.5..=3.0]))
		);
		assert_eq!(
			parse_with_names(literal, "{1.0} union 2.5..3.0").0,
			Literal::FloatSet(RangeList::from_iter([1.0..=1.0, 2.5..=3.0]))
		);
	}

	#[test]
	fn identifier_literal() {
		let (actual, names) = parse_with_names(literal, "some_name");
		assert_eq!(actual, Literal::Reference(name_id(&names, "some_name")));

		let (actual, names) = parse_with_names(literal, "_some_name");
		assert_eq!(actual, Literal::Reference(name_id(&names, "_some_name")));

		let (actual, names) = parse_with_names(literal, "_SomeName283");
		assert_eq!(actual, Literal::Reference(name_id(&names, "_SomeName283")));
	}

	#[test]
	fn int_literal() {
		assert_eq!(parse_with_names(literal, "0").0, Literal::Int(0));
		assert_eq!(parse_with_names(literal, "420").0, Literal::Int(420));
		assert_eq!(parse_with_names(literal, "-38").0, Literal::Int(-38));
		assert_eq!(
			parse_with_names(literal, "0xff32a").0,
			Literal::Int(0xff32a)
		);
		assert_eq!(
			parse_with_names(literal, "-0xadc20").0,
			Literal::Int(-0xadc20)
		);
		assert_eq!(
			parse_with_names(literal, "0o12356").0,
			Literal::Int(0o12356)
		);
		assert_eq!(parse_with_names(literal, "-0o230").0, Literal::Int(-0o230));
	}

	#[test]
	fn int_set_literal() {
		assert_eq!(
			parse_with_names(literal, "1..5").0,
			Literal::IntSet(RangeList::from(1..=5))
		);
		assert_eq!(
			parse_with_names(literal, "{1, 4, 6}").0,
			Literal::IntSet(RangeList::from_iter([1..=1, 4..=4, 6..=6]))
		);
		assert_eq!(
			parse_with_names(literal, "1..2 union 4..6").0,
			Literal::IntSet(RangeList::from_iter([1..=2, 4..=6]))
		);
		assert_eq!(
			parse_with_names(literal, "{1} union 4..5").0,
			Literal::IntSet(RangeList::from_iter([1..=1, 4..=5]))
		);
	}
}
