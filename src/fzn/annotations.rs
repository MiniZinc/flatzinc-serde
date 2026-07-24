//! Parser helpers for FlatZinc annotations.

use std::fmt::Display;

use winnow::{
	Parser, Result,
	combinator::{alt, delimited, opt, preceded, repeat, separated},
};

use crate::{
	fzn::{Stream, identifier, intern_parsed_identifier, literal, token},
	intermediate::{
		Annotation, AnnotationArgument, AnnotationCall, AnnotationLiteral, Argument, Literal,
		NameId,
	},
};

/// Semantic flags projected out of special FlatZinc annotations.
#[derive(Default)]
pub(crate) struct AnnotationFlags {
	/// Whether the variable is defined by a constraint.
	pub(crate) defined: bool,
	/// Whether the variable was additionally introduced by the compiler.
	pub(crate) introduced: bool,
	/// Whether the declaration should appear in the output.
	pub(crate) output: bool,
}

/// Parse an annotation.
///
/// ```bnf
/// <annotation> ::= <identifier>
///                | <identifier> "(" <ann-expr> "," ... ")"
/// ```
pub(super) fn annotation<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<(&'a str, Option<Vec<AnnotationArgument<Identifier>>>)>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	preceded(
		token("::"),
		(
			identifier,
			opt(delimited(
				token('('),
				separated(0.., token(annotation_argument), token(',')),
				token(')'),
			)),
		),
	)
	.parse_next(input)
}

/// Parse an annotation argument.
///
/// ```bnf
/// <ann-expr> := <basic-ann-expr>
///             | "[" [ <basic-ann-expr> "," ... ] "]"
/// ```
fn annotation_argument<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<AnnotationArgument<Identifier>>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	alt((
		annotation_literal.map(Argument::Literal),
		delimited(
			token('['),
			separated(0.., token(annotation_literal), token(',')),
			token(']'),
		)
		.map(Argument::Array),
	))
	.parse_next(input)
}

/// Parse a nested annotation call.
///
/// This does not have an analogue in the FZN grammar. It is only used to parse
/// annotation arguments that are nested annotation calls.
///
/// ```bnf
/// <ann-call> ::= <identifier> "(" <ann-expr> "," ... ")"
/// ```
fn annotation_call<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<AnnotationCall<Identifier>>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	let (id, args) = (
		identifier,
		delimited(
			token('('),
			separated(0.., token(annotation_argument), token(',')),
			token(')'),
		),
	)
		.parse_next(input)?;

	Ok(AnnotationCall {
		id: intern_parsed_identifier(input, id)?,
		args,
	})
}

/// Build a pending annotation from parsed identifier and optional arguments.
fn annotation_from_parts<Identifier, F, E>(
	input: &mut Stream<'_, Identifier, F>,
	ident: &str,
	args: Option<Vec<AnnotationArgument<Identifier>>>,
) -> Result<Annotation<Identifier>>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	let id = intern_parsed_identifier(input, ident)?;
	Ok(match args {
		Some(args) => Annotation::Call(AnnotationCall { id, args }),
		None => Annotation::Atom(id),
	})
}

/// Parses an annotation literal (or basic annotation expression).
///
/// ```bnf
/// <basic-ann-expr> := <basic-literal-expr>
///                   | <var-par-identifier>
///                   | <string-literal>
///                   | <annotation>
/// ```
fn annotation_literal<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<AnnotationLiteral<Identifier>>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	enum Parsed<Identifier> {
		Annotation(AnnotationCall<Identifier>),
		Literal(Literal),
	}

	let parsed = alt((
		annotation_call.map(Parsed::Annotation),
		literal.map(Parsed::Literal),
	))
	.parse_next(input)?;

	Ok(match parsed {
		Parsed::Annotation(annotation) => AnnotationLiteral::Annotation(annotation),
		Parsed::Literal(literal) => literal.into(),
	})
}

/// Parses the annotations for a constraint, returning optionally the identifier
/// of the defined variable and a list of annotations.
pub(super) fn constraint_annotations<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<(Option<NameId>, Vec<Annotation<Identifier>>)>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	let anns: Vec<(&str, Option<Vec<AnnotationArgument<Identifier>>>)> =
		repeat(0.., annotation).parse_next(input)?;
	let mut defines = None;
	let mut parsed = Vec::with_capacity(anns.len());

	for (ident, args) in anns {
		match (ident, args) {
			("defines_var", Some(mut args)) if args.len() == 1 => {
				if let Argument::Literal(AnnotationLiteral::Literal(Literal::Reference(name))) =
					args.remove(0)
				{
					defines = Some(name);
					continue;
				}
				parsed.push(annotation_from_parts(input, ident, Some(args))?);
			}
			(other_ident, other_args) => {
				parsed.push(annotation_from_parts(input, other_ident, other_args)?)
			}
		}
	}

	Ok((defines, parsed))
}

/// Parses a general list of annotations.
pub(super) fn general_annotations<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<Vec<Annotation<Identifier>>>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	let anns: Vec<(&str, Option<Vec<AnnotationArgument<Identifier>>>)> =
		repeat(0.., annotation).parse_next(input)?;
	anns.into_iter()
		.map(|(ident, args)| annotation_from_parts(input, ident, args))
		.collect()
}

/// Parses the annotations for a variable declaration, returning flags for
/// standard annotations and a list of other annotations.
pub(super) fn variable_annotations<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<(AnnotationFlags, Vec<Annotation<Identifier>>)>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	let anns: Vec<(&str, Option<Vec<AnnotationArgument<Identifier>>>)> =
		repeat(0.., annotation).parse_next(input)?;
	let mut flags = AnnotationFlags::default();
	let mut parsed = Vec::with_capacity(anns.len());

	for (ident, args) in anns {
		match (ident, args) {
			("is_defined_var", None) => flags.defined = true,
			("var_is_introduced", None) => flags.introduced = true,
			("output_var", None) => flags.output = true,
			("output_array", Some(_)) => flags.output = true,
			(other_ident, other_args) => {
				parsed.push(annotation_from_parts(input, other_ident, other_args)?)
			}
		}
	}

	Ok((flags, parsed))
}

#[cfg(test)]
mod tests {
	use rangelist::RangeList;

	use crate::{
		fzn::{
			general_annotations,
			tests::{annotation_identifier, parse_with_names},
		},
		intermediate::{Annotation, AnnotationCall, AnnotationLiteral, Argument, Literal},
	};

	#[test]
	fn annotation_call_with_array_argument() {
		let (actual, _) = parse_with_names(
			general_annotations,
			":: some_annotation([other_annotation(5), 3.4])",
		);
		assert_eq!(
			actual,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![Argument::Array(vec![
					AnnotationLiteral::Annotation(AnnotationCall {
						id: "other_annotation".to_owned(),
						args: vec![Argument::Literal(AnnotationLiteral::Literal(Literal::Int(
							5
						),))],
					}),
					AnnotationLiteral::Literal(Literal::Float(3.4)),
				])],
			})],
		);
	}

	#[test]
	fn annotation_call_with_literal_argument() {
		let (actual, names) =
			parse_with_names(general_annotations, ":: some_annotation(other_annotation)");
		assert_eq!(
			actual,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![Argument::Literal(annotation_identifier(
					&names,
					"other_annotation"
				),)],
			})],
		);

		let (actual, _) = parse_with_names(general_annotations, ":: some_annotation(1..5)");
		assert_eq!(
			actual,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![Argument::Literal(AnnotationLiteral::Literal(
					Literal::IntSet(RangeList::from(1..=5))
				))],
			})],
		);
	}

	#[test]
	fn annotation_call_with_nested_annotation_call_argument() {
		let (actual, _) = parse_with_names(
			general_annotations,
			":: some_annotation(other_annotation(5))",
		);
		assert_eq!(
			actual,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![Argument::Literal(AnnotationLiteral::Annotation(
					AnnotationCall {
						id: "other_annotation".to_owned(),
						args: vec![Argument::Literal(AnnotationLiteral::Literal(Literal::Int(
							5
						),))],
					}
				),)],
			})],
		);

		let (actual, _) = parse_with_names(
			general_annotations,
			":: some_annotation(another_annotation ())",
		);
		assert_eq!(
			actual,
			vec![Annotation::Call(AnnotationCall {
				id: "some_annotation".to_owned(),
				args: vec![Argument::Literal(AnnotationLiteral::Annotation(
					AnnotationCall {
						id: "another_annotation".to_owned(),
						args: vec![],
					}
				),)],
			})],
		);
	}

	#[test]
	fn atom_annotation() {
		let (actual, _) = parse_with_names(general_annotations, ":: output_var");
		assert_eq!(actual, vec![Annotation::Atom("output_var".to_owned())]);
	}
}
