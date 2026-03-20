//! Parser for an FZN annotation.

use std::fmt::{Debug, Display};

use winnow::{
	Parser, Result,
	combinator::{alt, delimited, opt, preceded, repeat, separated},
	error::{ContextError, FromExternalError},
};

use crate::{
	Annotation, AnnotationArgument, AnnotationCall, AnnotationLiteral, Literal,
	fzn::{Stream, identifier, identifier_raw, literal, token},
};

/// Semantic flags projected out of special FlatZinc annotations.
#[derive(Default)]
pub(crate) struct AnnotationFlags {
	/// Whether the variable is defined by a constraint
	pub(crate) defined: bool,
	/// Whether the variable was additionally introduced by the compiler.
	pub(crate) introduced: bool,
	/// Whether the variable is an output variable.
	pub(crate) output: bool,
}

/// Parse an annotation.
///
/// ```bnf
/// <annotation> ::= <identifier>
///                | <identifier> "(" <ann-expr> "," ... ")"
/// ```
pub(super) fn annotation<'a, 's, I, F, E>(
	input: &mut Stream<'a, 's, I, F>,
) -> Result<(&'a str, Option<Vec<AnnotationArgument<I>>>)>
where
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
	I: Clone + Debug,
{
	preceded(
		token("::"),
		(
			identifier_raw,
			opt(delimited(
				token('('),
				separated(0.., token(annotation_argument), token(',')),
				token(')'),
			)),
		),
	)
	.parse_next(input)
}

/// Parses an annotation argument (or annotation expression).
///
/// ```bnf
/// <ann-expr> := <basic-ann-expr>
///             | "[" [ <basic-ann-expr> "," ... ] "]"
/// ```
fn annotation_argument<'a, 's, I, F, E>(
	input: &mut Stream<'a, 's, I, F>,
) -> Result<AnnotationArgument<I>>
where
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
	I: Clone + Debug,
{
	alt((
		annotation_literal.map(AnnotationArgument::Literal),
		delimited(
			token('['),
			separated(0.., token(annotation_literal), token(',')),
			token(']'),
		)
		.map(AnnotationArgument::Array),
	))
	.parse_next(input)
}

/// Parses an annotation with arguments.
///
/// This does not have an analogue in the FZN grammar. It is only used to parse
/// annotation arguments that are nested annotation calls.
fn annotation_call<'a, 's, I, F, E>(input: &mut Stream<'a, 's, I, F>) -> Result<AnnotationCall<I>>
where
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
	I: Clone + Debug,
{
	(
		identifier,
		delimited(
			token('('),
			separated(0.., token(annotation_argument), token(',')),
			token(')'),
		),
	)
		.map(|(id, args)| AnnotationCall { id, args })
		.parse_next(input)
}

/// Parses an annotation literal (or basic annotation expression).
///
/// ```bnf
/// <basic-ann-expr> := <basic-literal-expr>
///                   | <var-par-identifier>
///                   | <string-literal>
///                   | <annotation>
/// ```
fn annotation_literal<'a, 's, I, F, E>(
	input: &mut Stream<'a, 's, I, F>,
) -> Result<AnnotationLiteral<I>>
where
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
	I: Clone + Debug,
{
	alt((
		annotation_call.map(AnnotationLiteral::Annotation),
		literal.map(AnnotationLiteral::BaseLiteral),
	))
	.parse_next(input)
}

/// Parses the annotations for a constraint, returning optionally the identifier
/// of the defined variable and a list of annotations.
pub(super) fn constraint_annotations<'a, 's, I, F, E>(
	input: &mut Stream<'a, 's, I, F>,
) -> Result<(Option<I>, Vec<Annotation<I>>)>
where
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
	I: Clone + Debug,
{
	let anns: Vec<(&str, Option<Vec<AnnotationArgument<I>>>)> =
		repeat(0.., annotation).parse_next(input)?;
	let mut defines = None;
	let mut parsed = Vec::with_capacity(anns.len());

	for (ident, args) in anns {
		match (ident, args) {
			("defines_var", Some(mut v))
				if v.len() == 1
					&& matches!(
						v[0],
						AnnotationArgument::Literal(AnnotationLiteral::BaseLiteral(
							Literal::Identifier(_)
						))
					) =>
			{
				let AnnotationArgument::Literal(AnnotationLiteral::BaseLiteral(
					Literal::Identifier(identifier),
				)) = v.remove(0)
				else {
					unreachable!()
				};
				defines = Some(identifier);
			}
			(ident, args) => {
				let ident = input
					.state
					.intern(ident)
					.map_err(|err| ContextError::from_external_error(input, err))?;
				parsed.push(if let Some(args) = args {
					Annotation::Call(AnnotationCall { id: ident, args })
				} else {
					Annotation::Atom(ident)
				});
			}
		}
	}

	Ok((defines, parsed))
}

/// Parses a general list of annotations.
pub(super) fn general_annotations<'a, 's, I, F, E>(
	input: &mut Stream<'a, 's, I, F>,
) -> Result<Vec<Annotation<I>>>
where
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
	I: Clone + Debug,
{
	let anns: Vec<(&str, Option<Vec<AnnotationArgument<I>>>)> =
		repeat(0.., annotation).parse_next(input)?;
	let mut parsed = Vec::with_capacity(anns.len());

	for (ident, args) in anns {
		let ident = input
			.state
			.intern(ident)
			.map_err(|err| ContextError::from_external_error(input, err))?;
		parsed.push(if let Some(args) = args {
			Annotation::Call(AnnotationCall { id: ident, args })
		} else {
			Annotation::Atom(ident)
		});
	}

	Ok(parsed)
}

/// Parses the annotations for a variable declaration, returning flags for
/// standard annotations and a list of other annotations.
pub(super) fn variable_annotations<I, F, E>(
	input: &mut Stream<'_, '_, I, F>,
) -> Result<(AnnotationFlags, Vec<Annotation<I>>)>
where
	F: FnMut(&str) -> std::result::Result<I, E>,
	E: Display,
	I: Clone + Debug,
{
	let anns: Vec<(&str, Option<Vec<AnnotationArgument<I>>>)> =
		repeat(0.., annotation).parse_next(input)?;
	let mut flags = AnnotationFlags::default();
	let mut parsed = Vec::with_capacity(anns.len());

	for (ident, args) in anns {
		match (ident, args) {
			("is_defined_var", None) => flags.defined = true,
			("var_is_introduced", None) => flags.introduced = true,
			("output_var", None) => flags.output = true,
			("output_array", Some(_)) => flags.output = true,
			(ident, args) => {
				let ident = input
					.state
					.intern(ident)
					.map_err(|err| ContextError::from_external_error(input, err))?;
				parsed.push(if let Some(args) = args {
					Annotation::Call(AnnotationCall { id: ident, args })
				} else {
					Annotation::Atom(ident)
				});
			}
		}
	}

	Ok((flags, parsed))
}

#[cfg(test)]
mod tests {
	use rangelist::RangeList;

	use crate::{
		Annotation, AnnotationArgument, AnnotationCall, AnnotationLiteral, Literal,
		fzn::{general_annotations, tests::check_parser},
	};

	#[test]
	fn annotation_call_with_array_argument() {
		check_parser(
			general_annotations,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![AnnotationArgument::Array(vec![
					AnnotationLiteral::Annotation(AnnotationCall {
						id: "other_annotation".to_owned(),
						args: vec![AnnotationArgument::Literal(AnnotationLiteral::BaseLiteral(
							Literal::Int(5),
						))],
					}),
					AnnotationLiteral::BaseLiteral(Literal::Float(3.4)),
				])],
			})],
			":: some_annotation([other_annotation(5), 3.4])",
		);
	}

	#[test]
	fn annotation_call_with_literal_argument() {
		check_parser(
			general_annotations,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![AnnotationArgument::Literal(AnnotationLiteral::BaseLiteral(
					Literal::Identifier("other_annotation".to_owned()),
				))],
			})],
			":: some_annotation(other_annotation)",
		);
		check_parser(
			general_annotations,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![AnnotationArgument::Literal(AnnotationLiteral::BaseLiteral(
					Literal::IntSet(RangeList::from(1..=5)),
				))],
			})],
			":: some_annotation(1..5)",
		);
	}

	#[test]
	fn annotation_call_with_nested_annotation_call_argument() {
		check_parser(
			general_annotations,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![AnnotationArgument::Literal(AnnotationLiteral::Annotation(
					AnnotationCall {
						id: "other_annotation".to_owned(),
						args: vec![AnnotationArgument::Literal(AnnotationLiteral::BaseLiteral(
							Literal::Int(5),
						))],
					},
				))],
			})],
			":: some_annotation(other_annotation(5))",
		);
		check_parser(
			general_annotations,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![AnnotationArgument::Literal(AnnotationLiteral::Annotation(
					AnnotationCall {
						id: "another_annotation".to_owned(),
						args: vec![],
					},
				))],
			})],
			":: some_annotation(another_annotation ())",
		);
	}

	#[test]
	fn atom_annotation() {
		check_parser(
			general_annotations,
			vec![Annotation::Atom("output_var".to_owned())],
			":: output_var",
		);
	}
}
