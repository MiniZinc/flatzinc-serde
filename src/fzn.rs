//! Parse the original `.fzn` file format.

mod annotations;
mod primitives;

use std::{fmt::Display, io::BufRead};

use annotations::*;
use primitives::*;
use winnow::{
	Parser, Result, Stateful,
	combinator::{alt, delimited, opt, preceded, separated, separated_pair},
	error::ContextError,
};

use crate::{
	FlatZinc, Type,
	error::FznParseError,
	intermediate::{
		self, Argument, Array, Constraint, Declaration, Literal, Method, NameId, ParserState,
		SolveObjective, Variable,
	},
};

/// Represents the current parsing phase.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ParsePhase {
	/// Accepting all items, possibly already parsed a predicate declaration.
	Predicates,
	/// Parsed a array/variable declaration item, no longer accepting predicate
	/// declarations.
	Declarations,
	/// Parsed a constraint item, no longer accepting predicate or
	/// array/variable declarations.
	Constraints,
	/// Parsed a solve item, no longer accepting any items.
	Solve,
}

/// Type used for the parser input and state.
pub(crate) type Stream<'source, Identifier, F> = Stateful<&'source str, ParserState<Identifier, F>>;

/// Parses a constraint argument.
///
/// ```bnf
/// <expr> ::= <basic-expr>
///          | <array-literal>
/// ```
fn argument<'a, Identifier, F>(input: &mut Stream<'a, Identifier, F>) -> Result<Argument> {
	alt((
		literal.map(Argument::Literal),
		delimited(
			token("["),
			separated(0.., token(literal), token(",")),
			token("]"),
		)
		.map(Argument::Array),
	))
	.parse_next(input)
}

/// Parse an array declaration.
///
/// ```bnf
/// <array-item> ::= "array" "[" <int-set> "]" "of" ["var"] <var-type>
///                  ":" <identifier> <annotations> "=" "[" <basic-expr> "," ... "]" ";"
/// ```
fn array_item<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<(NameId, Array<Identifier>, bool)>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	let _ = token("array").parse_next(input)?;
	let _ = delimited(token("["), interval_set(int), token("]")).parse_next(input)?;
	let _ = token("of").parse_next(input)?;
	let _ = opt(token("var")).parse_next(input)?;
	let _ = basic_variable_type.parse_next(input)?;

	let _ = token(":").parse_next(input)?;
	let id = token(identifier).parse_next(input)?;
	let name = input.state.intern_name(id);
	let (flags, ann) = variable_annotations.parse_next(input)?;
	let contents = preceded(token("="), delimited_list("[", literal, "]")).parse_next(input)?;
	let _ = token(";").parse_next(input)?;

	Ok((
		name,
		Array {
			contents,
			ann,
			defined: flags.defined,
			introduced: flags.introduced,
		},
		flags.output,
	))
}

/// Parse a basic parameter type.
///
/// ```bnf
/// <basic-par-type> ::= "bool"
///                    | "int"
///                    | "float"
///                    | "set" "of" "int"
/// ```
fn basic_parameter_type<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<Type> {
	alt((
		"bool".map(|_| Type::Bool),
		"int".map(|_| Type::Int(None)),
		"float".map(|_| Type::Float(None)),
		(token("set"), token("of"), token("int")).map(|_| Type::IntSet(None)),
	))
	.parse_next(input)
}

/// Parses the domain in a variable declaration.
///
/// Has no direct analogue in the grammar. However, it is essentially the
/// `<basic-var-type>` without the "var" token preceding it:
///
/// ```bnf
/// <basic-var-type> ::= "var" <basic-par-type>
///                    | "var" <int-literal> ".." <int-literal>
///                    | "var" "{" <int-literal> "," ... "}"
///                    | "var" <float-literal> ".." <float-literal>
///                    | "var" "set" "of" <int-literal> ".." <int-literal>
///                    | "var" "set" "of" "{" [ <int-literal> "," ... ] "}"
/// ```
fn basic_variable_type<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<Type> {
	alt((
		basic_parameter_type,
		preceded((token("set"), token("of")), set(int)).map(|values| Type::IntSet(Some(values))),
		set(int).map(|values| Type::Int(Some(values))),
		interval_set(float).map(|values| Type::Float(Some(values))),
	))
	.parse_next(input)
}

/// Parse a constraint item.
///
/// ```bnf
/// <constraint-item> ::= "constraint" <identifier> "(" [ <expr> "," ... ] ")" <annotations> ";"
/// ```
fn constraint<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<Constraint<Identifier>>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	let (_, id, args, (defines, ann), _) = (
		token("constraint"),
		token(identifier),
		delimited(
			token("("),
			separated(0.., token(argument), token(",")),
			token(")"),
		),
		constraint_annotations,
		token(";"),
	)
		.parse_next(input)?;
	Ok(Constraint {
		id: intern_parsed_identifier(input, id)?,
		args,
		ann,
		defines,
	})
}

/// Parse a declaration item.
///
/// ```bnf
/// <item> ::= <array-item>
///          | <var-decl-item>
/// ```
fn declaration<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<(NameId, Declaration<Identifier>, bool)>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	alt((
		array_item.map(|(name, arr, output)| (name, Declaration::Array(arr), output)),
		variable_declaration.map(|(name, var, output)| (name, Declaration::Variable(var), output)),
	))
	.parse_next(input)
}

/// Intern one parsed identifier while preserving `.fzn`-specific identifier
/// failures across parser-combinator unwinding.
fn intern_parsed_identifier<Identifier, F, E>(
	input: &mut Stream<'_, Identifier, F>,
	ident: &str,
) -> Result<Identifier>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	match input.state.intern_identifier(ident) {
		Ok(identifier) => Ok(identifier),
		Err(error) => {
			input.state.record_identifier_error(ident, error);
			Err(<ContextError as winnow::error::ParserError<_>>::from_input(
				input,
			))
		}
	}
}

/// Convert a parser failure into the richer public `.fzn` identifier error
/// when a frontend interner failure was recorded in the shared parser state.
fn map_parse_error<Identifier, F>(
	stream: &mut Stream<'_, Identifier, F>,
	error: ContextError,
) -> FznParseError {
	stream.state.take_identifier_error().map_or_else(
		|| FznParseError::SyntaxError(error.to_string()),
		|error| error.into(),
	)
}

/// Parse the `.fzn` source to a [`FlatZinc`] instance.
///
/// This is used by [`crate::FlatZinc::from_fzn`], which is the public entry
/// point for `.fzn` parsing.
pub(crate) fn parse<I, E>(source: impl BufRead) -> Result<FlatZinc<I>, FznParseError>
where
	I: Clone + for<'a> TryFrom<&'a str, Error = E>,
	E: Display,
{
	parse_with_interner(source, |s: &str| I::try_from(s))
}

/// Parse the `.fzn` source to a [`FlatZinc`] instance using a custom
/// identifier interner.
pub(crate) fn parse_with_interner<I, F, E>(
	mut source: impl BufRead,
	mut interner: F,
) -> Result<FlatZinc<I>, FznParseError>
where
	I: Clone,
	F: FnMut(&str) -> Result<I, E>,
	E: Display,
{
	let mut state = Some(ParserState::new(&mut interner));
	let mut constraints = Vec::new();
	let mut output = Vec::new();
	let mut solve = None;

	let mut buffer = Vec::new();
	let mut phase = ParsePhase::Predicates;

	loop {
		read_statement(&mut source, &mut buffer)?;
		if buffer.is_empty() {
			break;
		}

		let mut stream = Stateful {
			input: std::str::from_utf8(&buffer)?,
			state: state
				.take()
				.expect("parser state must be restored after each statement"),
		};

		token(())
			.parse_next(&mut stream)
			.map_err(|error| map_parse_error(&mut stream, error))?;
		if stream.input.is_empty() {
			state = Some(stream.state);
			continue;
		}

		if stream.input.starts_with("predicate") {
			if phase > ParsePhase::Predicates {
				return Err(FznParseError::SyntaxError(
					"predicate items must appear before declarations, constraints, and solve"
						.into(),
				));
			}
			token(predicate_item)
				.parse_next(&mut stream)
				.map_err(|error| map_parse_error(&mut stream, error))?;
			state = Some(stream.state);
			continue;
		}

		if stream.input.starts_with("constraint") {
			if phase > ParsePhase::Constraints {
				return Err(FznParseError::SyntaxError(
					"constraint items must appear before the solve item".into(),
				));
			}
			phase = ParsePhase::Constraints;
			let constraint = token(constraint)
				.parse_next(&mut stream)
				.map_err(|error| map_parse_error(&mut stream, error))?;
			constraints.push(constraint);
			state = Some(stream.state);
			continue;
		}

		if stream.input.starts_with("solve") {
			if phase > ParsePhase::Solve || solve.is_some() {
				return Err(FznParseError::MultipleSolveItems);
			}
			phase = ParsePhase::Solve;
			let solve_objective = token(solve_objective)
				.parse_next(&mut stream)
				.map_err(|error| map_parse_error(&mut stream, error))?;
			solve = Some(solve_objective);
			state = Some(stream.state);
			continue;
		}

		if phase > ParsePhase::Declarations {
			return Err(FznParseError::SyntaxError(
				"declarations must appear before constraints and the solve item".into(),
			));
		}
		phase = ParsePhase::Declarations;

		let (name, declaration, is_output) = token(declaration)
			.parse_next(&mut stream)
			.map_err(|error| map_parse_error(&mut stream, error))?;
		if is_output {
			output.push(name);
		}
		stream.state.define_name(name, declaration)?;
		state = Some(stream.state);
	}

	let (names, interner) = state
		.expect("parser state must be present after parsing")
		.into_parts();

	let pending = intermediate::FlatZinc {
		names,
		constraints,
		output,
		solve: solve.ok_or(FznParseError::MissingSolveItem)?,
		version: "1.0".to_owned(),
	};
	FlatZinc::from_intermediate(pending, interner).map_err(|err| err.into())
}

/// Parses a predicate item.
///
/// ```bnf
/// <predicate-item> ::= "predicate" <identifier> "(" [ <pred-param-type> : <identifier> "," ... ] ")" ";"
/// ```
fn predicate_item<'a, Identifier, F>(input: &mut Stream<'a, Identifier, F>) -> Result<()> {
	(
		token("predicate"),
		token(identifier),
		delimited_list("(", predicate_parameter, ")"),
		token(";"),
	)
		.map(|_| ())
		.parse_next(input)
}

/// Parse a predicate parameter.
///
/// Has no named equivalent in the FlatZinc grammar.
///
/// ```bnf
/// <pred-param-type> ":" <identifier>
/// ```
fn predicate_parameter<'a, Identifier, F>(input: &mut Stream<'a, Identifier, F>) -> Result<()> {
	separated_pair(
		token(predicate_parameter_type),
		token(":"),
		token(identifier),
	)
	.map(|_| ())
	.parse_next(input)
}

/// Parse a predicate parameter type.
///
/// ```bnf
/// <pred-param-type> ::= <basic-pred-param-type>
///                     | "array" "[" <pred-index-set> "]" "of" <basic-pred-param-type>
///
/// <basic-pred-param-type> ::= <basic-par-type>
///                           | <basic-var-type>
///                           | <int-literal> ".." <int-literal>
///                           | <float-literal> ".." <float-literal>
///                           | "{" <int-literal> "," ... "}"
///                           | "set" "of" "float"
///                           | "set" "of" <set-float-literal>
///                           | "set" "of" <set-int-literal>
/// ```
fn predicate_parameter_type<Identifier, F>(input: &mut Stream<'_, Identifier, F>) -> Result<()> {
	fn basic_predicate_parameter_type<Identifier, F>(
		input: &mut Stream<'_, Identifier, F>,
	) -> Result<()> {
		alt((
			basic_parameter_type.map(|_| ()),
			(token("set"), token("of"), token("float")).map(|_| ()),
			preceded(token("var"), basic_variable_type).map(|_| ()),
			set(int).map(|_| ()),
			interval_set(float).map(|_| ()),
			preceded((token("set"), token("of"), token("int")), set(int)).map(|_| ()),
			preceded((token("set"), token("of"), token("float")), set(float)).map(|_| ()),
		))
		.parse_next(input)
	}

	alt((
		basic_predicate_parameter_type,
		(
			token("array"),
			delimited(
				token("["),
				alt((
					token("int").map(|_| ()),
					token(interval_set(int)).map(|_| ()),
				)),
				token("]"),
			),
			token("of"),
			basic_predicate_parameter_type,
		)
			.map(|_| ()),
	))
	.parse_next(input)
}

/// Read a single FlatZinc statement, stopping at a `;` that is outside of
/// comments.
fn read_statement(source: &mut impl BufRead, buffer: &mut Vec<u8>) -> Result<(), FznParseError> {
	buffer.clear();

	enum CommentState {
		Normal,
		Line,
		Block,
		BlockStar,
	}
	let mut state = CommentState::Normal;
	let mut index = 0;

	loop {
		let read = source.read_until(b';', buffer)?;
		if read == 0 {
			return Ok(());
		}

		while index < buffer.len() {
			let byte = buffer[index];

			match state {
				CommentState::Normal => match byte {
					b'%' => {
						state = CommentState::Line;
						index += 1;
					}
					b'/' if index + 1 < buffer.len() && buffer[index + 1] == b'*' => {
						state = CommentState::Block;
						index += 2;
					}
					b';' => return Ok(()),
					_ => index += 1,
				},
				CommentState::Line => {
					if byte == b'\n' {
						state = CommentState::Normal;
					}
					index += 1;
				}
				CommentState::Block => {
					if byte == b'*' {
						state = CommentState::BlockStar;
					}
					index += 1;
				}
				CommentState::BlockStar => {
					state = if byte == b'/' {
						CommentState::Normal
					} else if byte == b'*' {
						CommentState::BlockStar
					} else {
						CommentState::Block
					};
					index += 1;
				}
			}
		}

		if buffer.last() != Some(&b';') {
			return Ok(());
		}
	}
}

/// Parse a solve item.
///
/// ```bnf
/// <solve-item> ::= "solve" <annotations> "satisfy" ";"
///                | "solve" <annotations> "minimize" <identifier> ";"
///                | "solve" <annotations> "maximize" <identifier> ";"
/// ```
fn solve_objective<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<SolveObjective<Identifier>>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	enum ParsedMethod<'a> {
		Satisfy,
		Minimize(&'a str),
		Maximize(&'a str),
	}

	let parsed = (
		token("solve"),
		general_annotations,
		alt((
			token("satisfy").map(|_| ParsedMethod::Satisfy),
			preceded(token("minimize"), token(identifier)).map(ParsedMethod::Minimize),
			preceded(token("maximize"), token(identifier)).map(ParsedMethod::Maximize),
		)),
		token(";"),
	)
		.parse_next(input)?;

	let (_, ann, method, _) = parsed;
	let method = match method {
		ParsedMethod::Satisfy => Method::Satisfy,
		ParsedMethod::Minimize(name) => {
			Method::Minimize(Literal::Reference(input.state.intern_name(name)))
		}
		ParsedMethod::Maximize(name) => {
			Method::Maximize(Literal::Reference(input.state.intern_name(name)))
		}
	};
	Ok(SolveObjective { method, ann })
}

/// Parse a variable declaration item for use in the top-level declaration
/// stream.
///
/// ```bnf
/// <var-decl-item> ::= "var" <var-type> ":" <identifier> <annotations> ";"
/// ```
fn variable_declaration<'a, Identifier, F, E>(
	input: &mut Stream<'a, Identifier, F>,
) -> Result<(NameId, Variable<Identifier>, bool)>
where
	Identifier: Clone,
	F: FnMut(&str) -> std::result::Result<Identifier, E>,
	E: Display,
{
	let (_, ty, _, name, (flags, ann), _) = (
		token("var"),
		token(basic_variable_type),
		token(":"),
		token(identifier),
		variable_annotations,
		token(";"),
	)
		.parse_next(input)?;
	let name = input.state.intern_name(name);

	Ok((
		name,
		Variable {
			ty,
			ann,
			defined: flags.defined,
			introduced: flags.introduced,
		},
		flags.output,
	))
}

#[cfg(test)]
mod tests {
	macro_rules! test_file {
		($file: ident) => {
			#[test]
			fn $file() {
				let s = format!("./corpus/fzn/{}.fzn", stringify!($file));
				let path = std::path::Path::new(&s);
				let fzn_file = File::open(path).unwrap();
				let fzn_reader = BufReader::new(fzn_file);
				let actual: FlatZinc = FlatZinc::from_fzn(fzn_reader).unwrap();

				let expected = expect_test::expect_file![&format!(
					"../corpus/fzn/{}.expected",
					stringify!($file)
				)];
				expected.assert_eq(&actual.to_string());
			}
		};
	}

	use std::{
		convert::Infallible,
		fmt::Debug,
		fs::File,
		io::{BufReader, Cursor},
		sync::Arc,
	};

	use rangelist::RangeList;
	use ustr::Ustr;
	use winnow::{Parser, Stateful};

	use crate::{
		FlatZinc, FznParseError, LinkError, NamedRef, Type,
		fzn::{
			Stream, array_item, constraint, predicate_item, solve_objective, variable_declaration,
		},
		intermediate::{
			Annotation, AnnotationArgument, AnnotationCall, AnnotationLiteral, Argument, Array,
			Constraint, Literal, Method, NameId, NameStore, ParserState, SolveObjective, Variable,
		},
	};

	#[test]
	fn scalar_parameter_declarations_are_rejected() {
		let err = FlatZinc::<String>::from_fzn(Cursor::new("int: y = 5;\nsolve satisfy;"))
			.expect_err("expected scalar parameter declaration to be rejected");

		assert!(matches!(err, FznParseError::SyntaxError(_)));
	}

	pub(crate) fn annotation_identifier(
		names: &NameStore<String>,
		name: &str,
	) -> AnnotationLiteral<String> {
		AnnotationLiteral::Reference(name_id(names, name))
	}

	#[test]
	fn arrays_and_variables_are_linked_in_public_ast() {
		let fzn = FlatZinc::<String>::from_fzn(Cursor::new(
			"var int: x;\narray [1..2] of var int: xs = [x, x];\nsolve satisfy;",
		))
		.expect("failed to parse model with array references");

		assert!(has_variable(&fzn, "x"));
		assert!(has_array(&fzn, "xs"));
		assert_eq!(fzn.arrays[0].contents.len(), 2);
	}

	#[test]
	fn arrays_are_parsed_and_promoted() {
		let (actual, names) = parse_with_names(
			array_item,
			"array [1..2] of var int: xs :: output_array([1..2]) :: var_is_introduced = [x, y];",
		);
		assert_eq!(
			actual,
			(
				name_id(&names, "xs"),
				Array {
					contents: vec![reference(&names, "x"), reference(&names, "y")],
					ann: vec![],
					defined: false,
					introduced: true,
				},
				true,
			),
		);

		let fzn = FlatZinc::<String>::from_fzn(Cursor::new(
			"var int: x;\nvar int: y;\narray [1..2] of var int: xs :: output_array([1..2]) = [x, y];\nsolve satisfy;",
		))
		.expect("failed to parse output array model");
		assert_eq!(output_names(&fzn), vec!["xs"]);
	}

	#[test]
	fn basic_constraint_with_array_argument() {
		let (actual, names) = parse_with_names(constraint, "constraint all_different([x, y]);");
		assert_eq!(
			actual,
			Constraint {
				id: "all_different".into(),
				args: vec![Argument::Array(vec![
					reference(&names, "x"),
					reference(&names, "y"),
				])],
				defines: None,
				ann: vec![],
			},
		);
	}

	#[test]
	fn basic_constraint_with_identifier_arguments() {
		let (actual, names) = parse_with_names(constraint, "constraint int_lt(x, y);");
		assert_eq!(
			actual,
			Constraint {
				id: "int_lt".into(),
				args: vec![
					Argument::Literal(reference(&names, "x")),
					Argument::Literal(reference(&names, "y")),
				],
				defines: None,
				ann: vec![],
			},
		);
	}

	#[test]
	fn basic_constraint_with_identifier_arguments_and_annotation() {
		let (actual, names) =
			parse_with_names(constraint, "constraint int_lt(x, y) :: domain_consistent;");
		assert_eq!(
			actual,
			Constraint {
				id: "int_lt".into(),
				args: vec![
					Argument::Literal(reference(&names, "x")),
					Argument::Literal(reference(&names, "y")),
				],
				defines: None,
				ann: vec![Annotation::Atom("domain_consistent".to_owned())],
			},
		);
	}

	#[test]
	fn constraint_defines_var_annotation_is_promoted() {
		let (actual, names) =
			parse_with_names(constraint, "constraint bool2int(b, x) :: defines_var(x);");
		assert_eq!(
			actual,
			Constraint {
				id: "bool2int".into(),
				args: vec![
					Argument::Literal(reference(&names, "b")),
					Argument::Literal(reference(&names, "x")),
				],
				defines: Some(name_id(&names, "x")),
				ann: vec![],
			},
		);
	}

	#[test]
	fn constraint_keeps_non_semantic_annotations_after_promotion() {
		let (actual, names) = parse_with_names(
			constraint,
			"constraint int_lin_eq([400, 450, -1], [b, c, obj], 0) :: defines_var(obj) :: ctx_pos;",
		);
		assert_eq!(
			actual,
			Constraint {
				id: "int_lin_eq".into(),
				args: vec![
					Argument::Array(vec![Literal::Int(400), Literal::Int(450), Literal::Int(-1),]),
					Argument::Array(vec![
						reference(&names, "b"),
						reference(&names, "c"),
						reference(&names, "obj"),
					]),
					Argument::Literal(Literal::Int(0)),
				],
				defines: Some(name_id(&names, "obj")),
				ann: vec![Annotation::Atom("ctx_pos".to_owned())],
			},
		);
	}

	fn has_array<Identifier>(fzn: &FlatZinc<Identifier>, name: &str) -> bool {
		fzn.arrays.iter().any(|array| array.name == name)
	}

	fn has_variable<Identifier>(fzn: &FlatZinc<Identifier>, name: &str) -> bool {
		fzn.variables.iter().any(|variable| variable.name == name)
	}

	#[test]
	fn introduced_array_of_variables() {
		let (actual, names) = parse_with_names(
			array_item,
			"array [1..3] of var int: X_INTRODUCED_1_ ::var_is_introduced  = [x,y,z];",
		);
		assert_eq!(
			actual,
			(
				name_id(&names, "X_INTRODUCED_1_"),
				Array {
					contents: vec![
						reference(&names, "x"),
						reference(&names, "y"),
						reference(&names, "z"),
					],
					ann: vec![],
					defined: false,
					introduced: true,
				},
				false,
			),
		);
	}

	pub(crate) fn name_id(names: &NameStore<String>, expected: &str) -> NameId {
		names
			.iter()
			.find_map(|(id, entry)| (entry.name.as_ref() == expected).then_some(id))
			.unwrap_or_else(|| panic!("expected interned name `{expected}`"))
	}

	fn output_names<Identifier>(fzn: &FlatZinc<Identifier>) -> Vec<&str> {
		fzn.output.iter().map(NamedRef::name).collect()
	}

	#[test]
	fn output_variable_annotation_is_promoted() {
		let (actual, names) = parse_with_names(
			variable_declaration,
			"var int: x :: output_var :: var_is_introduced;",
		);
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Int(None),
					ann: vec![],
					defined: false,
					introduced: true,
				},
				true,
			),
		);

		let fzn =
			FlatZinc::<String>::from_fzn(Cursor::new("var int: x :: output_var;\nsolve satisfy;"))
				.expect("failed to parse output variable model");
		assert_eq!(output_names(&fzn), vec!["x"]);
	}

	#[test]
	fn scalar_variable_assignments_are_rejected() {
		let err = FlatZinc::<String>::from_fzn(Cursor::new("var int: x = 1;\nsolve satisfy;"))
			.expect_err("expected scalar variable assignment to be rejected");

		assert!(matches!(err, FznParseError::SyntaxError(_)));
	}

	#[test]
	fn parse_allows_comments() {
		let fzn = FlatZinc::<String>::from_fzn(Cursor::new(
			"/* leading block comment with ; inside */\nvar int: x;\nconstraint int_eq(x, x) /* inline block ; comment */;\n% before solve\nsolve satisfy;",
		))
		.expect("failed to parse model with comments");

		assert!(has_variable(&fzn, "x"));
		assert_eq!(fzn.constraints.len(), 1);
		assert_eq!(fzn.solve.method, crate::Method::Satisfy);
	}

	#[test]
	fn parse_rejects_invalid_item_ordering() {
		let error =
			FlatZinc::<String>::from_fzn(Cursor::new("solve satisfy;\nconstraint int_eq(x, x);"))
				.expect_err("expected parse to reject constraints after solve");
		assert!(matches!(error, FznParseError::SyntaxError(_)));

		let error = FlatZinc::<String>::from_fzn(Cursor::new(
			"constraint int_eq(x, x);\nvar int: x;\nsolve satisfy;",
		))
		.expect_err("expected parse to reject declarations after constraints");
		assert!(matches!(error, FznParseError::SyntaxError(_)));

		let error = FlatZinc::<String>::from_fzn(Cursor::new(
			"var int: x;\npredicate p(var int: y);\nsolve satisfy;",
		))
		.expect_err("expected parse to reject predicates after declarations");
		assert!(matches!(error, FznParseError::SyntaxError(_)));
	}

	#[test]
	fn parse_rejects_multiple_solve_items() {
		let error = FlatZinc::<String>::from_fzn(Cursor::new("solve satisfy;\nsolve minimize x;"))
			.expect_err("expected parse to reject multiple solve items");
		assert!(matches!(error, FznParseError::MultipleSolveItems));
	}

	#[test]
	fn parse_reports_identifier_parse_errors() {
		#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
		struct XIdentifier(String);

		impl<'a> TryFrom<&'a str> for XIdentifier {
			type Error = &'static str;

			fn try_from(s: &'a str) -> Result<Self, Self::Error> {
				s.starts_with('x')
					.then(|| Self(s.to_owned()))
					.ok_or("identifier must start with x")
			}
		}

		let error = FlatZinc::<XIdentifier>::from_fzn(Cursor::new(
			"var int: x;\nconstraint int_eq(x, x);\nsolve satisfy;",
		))
		.expect_err("expected parse to reject unsupported identifiers");
		assert!(matches!(
			error,
			FznParseError::LinkError(LinkError::IdentifierError { .. })
		));
	}

	#[test]
	fn parse_supports_custom_identifier_types() {
		let fzn = FlatZinc::<Ustr>::from_fzn(Cursor::new(
			"var int: x :: output_var;\nconstraint int_eq(x, x);\nsolve satisfy;",
		))
		.expect("failed to parse model with Ustr identifiers");

		assert!(has_variable(&fzn, "x"));
		assert_eq!(output_names(&fzn), vec!["x"]);
		assert_eq!(fzn.constraints[0].id, Ustr::from("int_eq"));
	}

	#[test]
	fn parse_supports_custom_interner_functions() {
		#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
		struct Interned(String);

		let fzn = FlatZinc::<Interned>::from_fzn_with_interner(
			Cursor::new("var int: x :: output_var;\nconstraint int_eq(x, x);\nsolve satisfy;"),
			|s| Ok::<_, Infallible>(Interned(format!("id:{s}"))),
		)
		.expect("failed to parse model with custom interner");

		assert!(has_variable(&fzn, "x"));
		assert_eq!(output_names(&fzn), vec!["x"]);
		assert_eq!(fzn.constraints[0].id, Interned("id:int_eq".to_owned()));
	}

	pub(crate) fn parse_with_names<'s, P, O>(
		mut parser: P,
		input: &'s str,
	) -> (O, NameStore<String>)
	where
		P: Parser<
				Stream<'s, String, fn(&str) -> Result<String, Infallible>>,
				O,
				winnow::error::ContextError,
			>,
		O: Debug,
	{
		let interner: fn(&str) -> Result<String, Infallible> = |input| Ok(input.to_owned());
		let mut stream = Stateful {
			input,
			state: ParserState::new(interner),
		};

		let actual = parser
			.parse_next(&mut stream)
			.unwrap_or_else(|err| panic!("parser returned unexpected error: {err:?}"));
		(actual, stream.state.names)
	}

	#[test]
	fn predicate_items_are_parsed_but_ignored() {
		let (actual, _) = parse_with_names(
			predicate_item,
			"predicate array_int_minimum(var int: m,array [int] of var int: x);",
		);
		assert_eq!(actual, ());
		let (actual, _) = parse_with_names(
			predicate_item,
			"predicate my_float_set_in(var float: x,set of float: y);",
		);
		assert_eq!(actual, ());
	}

	fn reference(names: &NameStore<String>, name: &str) -> Literal {
		Literal::Reference(name_id(names, name))
	}

	#[test]
	fn solve_annotations_round_trip_to_public_ast() {
		let fzn = FlatZinc::<String>::from_fzn(Cursor::new(
			"var int: x;\nsolve :: int_search([x], input_order, indomain_min, complete) maximize x;",
		))
		.expect("failed to parse solve annotations");

		assert_eq!(
			fzn.solve.method,
			crate::Method::Maximize(crate::Literal::Variable(Arc::clone(&fzn.variables[0])))
		);
		assert_eq!(
			fzn.solve.ann,
			vec![crate::Annotation::Call(crate::AnnotationCall {
				id: "int_search".to_owned(),
				args: vec![
					crate::AnnotationArgument::Array(vec![crate::AnnotationLiteral::Variable(
						Arc::downgrade(&fzn.variables[0]),
					)]),
					crate::AnnotationArgument::Literal(crate::AnnotationLiteral::Annotation(
						crate::Annotation::Atom("input_order".to_owned()),
					)),
					crate::AnnotationArgument::Literal(crate::AnnotationLiteral::Annotation(
						crate::Annotation::Atom("indomain_min".to_owned()),
					)),
					crate::AnnotationArgument::Literal(crate::AnnotationLiteral::Annotation(
						crate::Annotation::Atom("complete".to_owned()),
					)),
				],
			})]
		);
	}

	#[test]
	fn solve_items_are_parsed() {
		let (actual, names) = parse_with_names(solve_objective, "solve minimize w;");
		assert_eq!(
			actual,
			SolveObjective {
				method: Method::Minimize(reference(&names, "w")),
				ann: vec![],
			},
		);

		let (actual, names) = parse_with_names(
			solve_objective,
			"solve :: int_search([x, y, z], first_fail, indomain_split, complete) satisfy;",
		);
		assert_eq!(
			actual,
			SolveObjective {
				method: Method::Satisfy,
				ann: vec![Annotation::Call(AnnotationCall {
					id: "int_search".to_owned(),
					args: vec![
						AnnotationArgument::Array(vec![
							annotation_identifier(&names, "x"),
							annotation_identifier(&names, "y"),
							annotation_identifier(&names, "z"),
						]),
						AnnotationArgument::Literal(annotation_identifier(&names, "first_fail")),
						AnnotationArgument::Literal(annotation_identifier(
							&names,
							"indomain_split",
						)),
						AnnotationArgument::Literal(annotation_identifier(&names, "complete")),
					],
				})],
			},
		);
	}

	#[test]
	fn solve_optimize() {
		let (actual, names) = parse_with_names(solve_objective, "solve minimize w;");
		assert_eq!(
			actual,
			SolveObjective {
				method: Method::Minimize(reference(&names, "w")),
				ann: vec![],
			},
		);

		let (actual, names) = parse_with_names(solve_objective, "solve maximize w;");
		assert_eq!(
			actual,
			SolveObjective {
				method: Method::Maximize(reference(&names, "w")),
				ann: vec![],
			},
		);
	}

	#[test]
	fn solve_satisfy() {
		let (actual, _) = parse_with_names(solve_objective, "solve satisfy;");
		assert_eq!(
			actual,
			SolveObjective {
				method: Method::Satisfy,
				ann: vec![],
			},
		);
	}

	#[test]
	fn solve_with_annotations() {
		let (actual, names) = parse_with_names(
			solve_objective,
			"solve :: int_search(xs, input_order, indomain_min, complete) satisfy;",
		);
		assert_eq!(
			actual,
			SolveObjective {
				method: Method::Satisfy,
				ann: vec![Annotation::Call(AnnotationCall {
					id: "int_search".to_owned(),
					args: vec![
						AnnotationArgument::Literal(annotation_identifier(&names, "xs",)),
						AnnotationArgument::Literal(annotation_identifier(&names, "input_order",)),
						AnnotationArgument::Literal(annotation_identifier(&names, "indomain_min",)),
						AnnotationArgument::Literal(annotation_identifier(&names, "complete",)),
					],
				})],
			},
		);

		let (actual, names) = parse_with_names(
			solve_objective,
			"solve :: int_search([x, y, z], first_fail, indomain_split, complete) maximize x;",
		);
		assert_eq!(
			actual,
			SolveObjective {
				method: Method::Maximize(reference(&names, "x")),
				ann: vec![Annotation::Call(AnnotationCall {
					id: "int_search".to_owned(),
					args: vec![
						AnnotationArgument::Array(vec![
							annotation_identifier(&names, "x"),
							annotation_identifier(&names, "y"),
							annotation_identifier(&names, "z"),
						]),
						AnnotationArgument::Literal(annotation_identifier(&names, "first_fail",)),
						AnnotationArgument::Literal(annotation_identifier(
							&names,
							"indomain_split",
						)),
						AnnotationArgument::Literal(annotation_identifier(&names, "complete",)),
					],
				})],
			},
		);
	}

	#[test]
	fn some_parameter_array_items() {
		let (actual, names) =
			parse_with_names(array_item, "array [1..3] of int: some_param = [5, 3, 10];");
		assert_eq!(
			actual,
			(
				name_id(&names, "some_param"),
				Array {
					contents: vec![Literal::Int(5), Literal::Int(3), Literal::Int(10),],
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);

		let (actual, names) =
			parse_with_names(array_item, "array [1..2] of int: X_INTRODUCED_4_ = [-1,1];");
		assert_eq!(
			actual,
			(
				name_id(&names, "X_INTRODUCED_4_"),
				Array {
					contents: vec![Literal::Int(-1), Literal::Int(1),],
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);
	}

	#[test]
	fn variable_introduced_and_or_defined() {
		let (actual, names) =
			parse_with_names(variable_declaration, "var int: x :: var_is_introduced;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Int(None),
					ann: vec![],
					defined: false,
					introduced: true,
				},
				false,
			),
		);

		let (actual, names) =
			parse_with_names(variable_declaration, "var int: x :: is_defined_var;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Int(None),
					ann: vec![],
					defined: true,
					introduced: false,
				},
				false,
			),
		);

		let (actual, names) = parse_with_names(
			variable_declaration,
			"var bool: x :: is_defined_var :: var_is_introduced;",
		);
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Bool,
					ann: vec![],
					defined: true,
					introduced: true,
				},
				false,
			),
		);
	}

	#[test]
	fn variable_with_bounded_float_domain() {
		let (actual, names) = parse_with_names(variable_declaration, "var 1.0..5.5: x;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Float(Some(RangeList::from(1.0..=5.5))),
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);
	}

	#[test]
	fn variable_with_bounded_int_domain() {
		let (actual, names) = parse_with_names(variable_declaration, "var 1..5: x;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Int(Some(RangeList::from(1..=5))),
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);

		let (actual, names) = parse_with_names(variable_declaration, "var {1, 4, 6}: x;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Int(Some(RangeList::from_iter([1..=1, 4..=4, 6..=6]))),
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);
	}

	#[test]
	fn variable_with_int_set_domain() {
		let (actual, names) = parse_with_names(variable_declaration, "var set of 1..5: x;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::IntSet(Some(RangeList::from(1..=5))),
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);

		let (actual, names) = parse_with_names(variable_declaration, "var set of {1, 3}: x;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::IntSet(Some(RangeList::from_iter([1..=1, 3..=3]))),
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);
	}

	#[test]
	fn variable_with_named_domain() {
		let (actual, names) = parse_with_names(variable_declaration, "var int: x;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Int(None),
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);

		let (actual, names) = parse_with_names(variable_declaration, "var float: x;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Float(None),
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);

		let (actual, names) = parse_with_names(variable_declaration, "var set of int: x;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::IntSet(None),
					ann: vec![],
					defined: false,
					introduced: false,
				},
				false,
			),
		);
	}

	#[test]
	fn variables_are_parsed_with_domains_and_annotations() {
		let (actual, names) = parse_with_names(variable_declaration, "var 1..5: x :: mip;");
		assert_eq!(
			actual,
			(
				name_id(&names, "x"),
				Variable {
					ty: Type::Int(Some(RangeList::from(1..=5))),
					ann: vec![Annotation::Atom("mip".to_owned())],
					defined: false,
					introduced: false,
				},
				false,
			),
		);
	}

	test_file!(comments);
	test_file!(documentation_example);
	test_file!(empty_model);
	test_file!(float);
	test_file!(float_set);
	test_file!(nested_search);
	test_file!(predicates);
	test_file!(set_var);
}
