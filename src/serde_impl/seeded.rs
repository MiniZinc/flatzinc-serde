//! Seeded FlatZinc deserialization into `intermediate::FlatZinc` with custom
//! interning.

use std::{borrow::Cow, fmt::Display, marker::PhantomData};

use serde::{
	Deserialize,
	de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};

use super::{BaseType, VariableDomain};
use crate::{
	Type,
	intermediate::{
		self, Annotation, AnnotationArgument, AnnotationCall, AnnotationLiteral, Argument, Array,
		Constraint, Literal, Method, NameId, ParserState, SolveObjective, Variable,
	},
};

/// Numeric literal used to distinguish integer and floating-point set ranges.
#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
enum NumberValue {
	/// Integer endpoint.
	Int(i64),
	/// Floating-point endpoint.
	Float(f64),
}

/// Result of decoding an encapsulated FlatZinc set literal.
enum NumericSet {
	/// Integer set value.
	Int(crate::RangeList<i64>),
	/// Floating-point set value.
	Float(crate::RangeList<f64>),
}
/// Marker for single annotation deserialization.
struct ParseAnnotation;
/// Marker for single annotation-argument deserialization.
struct ParseAnnotationArgument;
/// Marker for annotation-arguments-sequence deserialization.
struct ParseAnnotationArguments;
/// Marker for single annotation-literal deserialization.
struct ParseAnnotationLiteral;
/// Marker for annotations-sequence deserialization.
struct ParseAnnotations;
/// Marker for single argument deserialization.
struct ParseArgument;
/// Marker for arguments-sequence deserialization.
struct ParseArguments;
/// Marker for single array deserialization.
struct ParseArray;
/// Marker for top-level arrays-object deserialization.
struct ParseArrays;
/// Marker for single constraint deserialization.
struct ParseConstraint;
/// Marker for constraints-sequence deserialization.
struct ParseConstraints;
/// Marker for single literal deserialization.
struct ParseLiteral;
/// Marker for literal-sequence deserialization.
struct ParseLiterals;
/// Marker for output-sequence deserialization.
struct ParseOutput;
/// Marker for solve-object deserialization.
struct ParseSolve;
/// Marker for single variable deserialization.
struct ParseVariable;

/// Marker for top-level variables-object deserialization.
struct ParseVariables;

/// Generic seed wrapper used for all stateful deserialization entry points in
/// this module.
struct Seed<'a, Identifier, F, Parser> {
	/// Shared parser state.
	state: &'a mut ParserState<Identifier, F>,
	/// Marker selecting the parser implementation.
	marker: PhantomData<Parser>,
}

/// Parse an annotation literal object once the first field name has already
/// been read.
fn deserialize_annotation_literal<'de, A, Identifier, F, E>(
	first_field: Cow<'_, str>,
	map: &mut A,
	state: &mut ParserState<Identifier, F>,
) -> Result<AnnotationLiteral<Identifier>, A::Error>
where
	A: MapAccess<'de>,
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	match first_field.as_ref() {
		"set" => {
			let ranges = map.next_value::<Vec<(NumberValue, NumberValue)>>()?;
			ensure_object_finished(map)?;
			match ranges.try_into().map_err(de::Error::custom)? {
				NumericSet::Int(ranges) => Ok(AnnotationLiteral::IntSet(ranges)),
				NumericSet::Float(ranges) => Ok(AnnotationLiteral::FloatSet(ranges)),
			}
		}
		"string" => {
			let string = map.next_value::<String>()?;
			ensure_object_finished(map)?;
			Ok(AnnotationLiteral::String(string))
		}
		"id" => {
			let raw = map.next_value::<Cow<'_, str>>()?;
			let mut id = Some(
				state
					.intern_identifier(raw.as_ref())
					.map_err(de::Error::custom)?,
			);
			let mut args = None;
			while let Some(field) = map.next_key::<Cow<'_, str>>()? {
				match field.as_ref() {
					"args" => {
						if args.is_some() {
							return Err(de::Error::duplicate_field("args"));
						}
						args = Some(map.next_value_seed(state.seed::<ParseAnnotationArguments>())?);
					}
					"id" => {
						if id.is_some() {
							return Err(de::Error::duplicate_field("id"));
						}
						let raw = map.next_value::<Cow<'_, str>>()?;
						id = Some(
							state
								.intern_identifier(raw.as_ref())
								.map_err(de::Error::custom)?,
						);
					}
					field => {
						return Err(de::Error::unknown_field(field, &["id", "args"]));
					}
				}
			}
			Ok(AnnotationLiteral::Annotation(AnnotationCall {
				id: id.ok_or_else(|| de::Error::missing_field("id"))?,
				args: args.ok_or_else(|| de::Error::missing_field("args"))?,
			}))
		}
		"args" => {
			let mut args = Some(map.next_value_seed(state.seed::<ParseAnnotationArguments>())?);
			let mut id = None;
			while let Some(field) = map.next_key::<Cow<'_, str>>()? {
				match field.as_ref() {
					"id" => {
						if id.is_some() {
							return Err(de::Error::duplicate_field("id"));
						}
						let raw = map.next_value::<Cow<'_, str>>()?;
						id = Some(
							state
								.intern_identifier(raw.as_ref())
								.map_err(de::Error::custom)?,
						);
					}
					"args" => {
						if args.is_some() {
							return Err(de::Error::duplicate_field("args"));
						}
						args = Some(map.next_value_seed(state.seed::<ParseAnnotationArguments>())?);
					}
					field => {
						return Err(de::Error::unknown_field(field, &["id", "args"]));
					}
				}
			}
			Ok(AnnotationLiteral::Annotation(AnnotationCall {
				id: id.ok_or_else(|| de::Error::missing_field("id"))?,
				args: args.ok_or_else(|| de::Error::missing_field("args"))?,
			}))
		}
		_ => Err(de::Error::custom(
			"expected a set, string, or annotation-call object",
		)),
	}
}

/// Ensure that a wrapper object contains no additional fields.
fn ensure_object_finished<'de, A>(map: &mut A) -> Result<(), A::Error>
where
	A: MapAccess<'de>,
{
	if map.next_key::<IgnoredAny>()?.is_some() {
		let _ = map.next_value::<IgnoredAny>()?;
		return Err(de::Error::custom(
			"unexpected extra fields in FlatZinc wrapper object",
		));
	}
	Ok(())
}

impl BaseType {
	/// Convert the raw JSON variable type representation into the public type.
	fn into_type(self, domain: Option<VariableDomain>) -> Result<Type, String> {
		match (self, domain) {
			(BaseType::Bool, None) => Ok(Type::Bool),
			(BaseType::Bool, Some(_)) => Err("bool variables cannot have a domain".to_owned()),
			(BaseType::Int, None) => Ok(Type::Int(None)),
			(BaseType::Int, Some(VariableDomain::Int(domain))) => Ok(Type::Int(Some(domain))),
			(BaseType::Int, Some(VariableDomain::Float(_))) => {
				Err("int variables require an int domain".to_owned())
			}
			(BaseType::Float, None) => Ok(Type::Float(None)),
			(BaseType::Float, Some(VariableDomain::Float(domain))) => Ok(Type::Float(Some(domain))),
			(BaseType::Float, Some(VariableDomain::Int(_))) => {
				Err("float variables require a float domain".to_owned())
			}
			(BaseType::IntSet, None) => Ok(Type::IntSet(None)),
			(BaseType::IntSet, Some(VariableDomain::Int(domain))) => Ok(Type::IntSet(Some(domain))),
			(BaseType::IntSet, Some(VariableDomain::Float(_))) => {
				Err("set of int variables require an int domain".to_owned())
			}
		}
	}
}

impl NumberValue {
	/// Convert one numeric endpoint to `f64`.
	fn into_f64(self) -> f64 {
		match self {
			NumberValue::Int(value) => value as f64,
			NumberValue::Float(value) => value,
		}
	}

	/// Convert one numeric endpoint to `i64`.
	fn into_i64(self) -> i64 {
		match self {
			NumberValue::Int(value) => value,
			NumberValue::Float(_) => {
				unreachable!("float endpoints are filtered before int conversion")
			}
		}
	}
}

impl TryFrom<Vec<(NumberValue, NumberValue)>> for NumericSet {
	type Error = String;

	fn try_from(ranges: Vec<(NumberValue, NumberValue)>) -> Result<Self, Self::Error> {
		if ranges.iter().any(|(start, end)| {
			matches!(start, NumberValue::Float(_)) || matches!(end, NumberValue::Float(_))
		}) {
			Ok(NumericSet::Float(
				ranges
					.into_iter()
					.map(|(start, end)| start.into_f64()..=end.into_f64())
					.collect(),
			))
		} else {
			Ok(NumericSet::Int(
				ranges
					.into_iter()
					.map(|(start, end)| start.into_i64()..=end.into_i64())
					.collect(),
			))
		}
	}
}

impl<Identifier, F> ParserState<Identifier, F> {
	/// Create a typed serde seed backed by this parser state.
	fn seed<Parser>(&mut self) -> Seed<'_, Identifier, F, Parser> {
		Seed::new(self)
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for ParserState<Identifier, F>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = (intermediate::FlatZinc<Identifier>, F);

	fn deserialize<D>(mut self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct ModelParts<Identifier> {
			constraints: Vec<Constraint<Identifier>>,
			output: Vec<NameId>,
			solve: Option<SolveObjective<Identifier>>,
			version: Option<String>,
		}

		struct ModelVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for ModelVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = ModelParts<Identifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a FlatZinc JSON object")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut seen_variables = false;
				let mut seen_arrays = false;
				let mut constraints = None;
				let mut output = None;
				let mut solve = None;
				let mut version = None;

				while let Some(field) = map.next_key::<Cow<'_, str>>()? {
					match field.as_ref() {
						"variables" => {
							if seen_variables {
								return Err(de::Error::duplicate_field("variables"));
							}
							seen_variables = true;
							map.next_value_seed(self.state.seed::<ParseVariables>())?;
						}
						"arrays" => {
							if seen_arrays {
								return Err(de::Error::duplicate_field("arrays"));
							}
							seen_arrays = true;
							map.next_value_seed(self.state.seed::<ParseArrays>())?;
						}
						"constraints" => {
							if constraints.is_some() {
								return Err(de::Error::duplicate_field("constraints"));
							}
							constraints =
								Some(map.next_value_seed(self.state.seed::<ParseConstraints>())?);
						}
						"output" => {
							if output.is_some() {
								return Err(de::Error::duplicate_field("output"));
							}
							output = Some(map.next_value_seed(self.state.seed::<ParseOutput>())?);
						}
						"solve" => {
							if solve.is_some() {
								return Err(de::Error::duplicate_field("solve"));
							}
							solve = Some(map.next_value_seed(self.state.seed::<ParseSolve>())?);
						}
						"version" => {
							if version.is_some() {
								return Err(de::Error::duplicate_field("version"));
							}
							version = Some(map.next_value()?);
						}
						_ => {
							let _ = map.next_value::<IgnoredAny>()?;
						}
					}
				}

				Ok(ModelParts {
					constraints: constraints.unwrap_or_default(),
					output: output.unwrap_or_default(),
					solve,
					version,
				})
			}
		}

		let parts = deserializer.deserialize_map(ModelVisitor {
			state: &mut self,
			marker: PhantomData,
		})?;
		let (names, interner) = self.into_parts();
		Ok((
			intermediate::FlatZinc {
				names,
				constraints: parts.constraints,
				output: parts.output,
				solve: parts
					.solve
					.ok_or_else(|| de::Error::missing_field("solve"))?,
				version: parts.version.unwrap_or_default(),
			},
			interner,
		))
	}
}

impl<'a, Identifier, F, Parser> Seed<'a, Identifier, F, Parser> {
	/// Create one typed seed backed by the shared parser state.
	fn new(state: &'a mut ParserState<Identifier, F>) -> Self {
		Self {
			state,
			marker: PhantomData,
		}
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseAnnotation>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = Annotation<Identifier>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct AnnotationVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for AnnotationVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = Annotation<Identifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("an annotation atom or annotation call")
			}

			fn visit_borrowed_str<E2>(self, value: &'de str) -> Result<Self::Value, E2>
			where
				E2: de::Error,
			{
				Ok(Annotation::Atom(
					self.state.intern_identifier(value).map_err(E2::custom)?,
				))
			}

			fn visit_str<E2>(self, value: &str) -> Result<Self::Value, E2>
			where
				E2: de::Error,
			{
				Ok(Annotation::Atom(
					self.state.intern_identifier(value).map_err(E2::custom)?,
				))
			}

			fn visit_string<E2>(self, value: String) -> Result<Self::Value, E2>
			where
				E2: de::Error,
			{
				Ok(Annotation::Atom(
					self.state.intern_identifier(&value).map_err(E2::custom)?,
				))
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut id = None;
				let mut args = None;

				while let Some(field) = map.next_key::<Cow<'_, str>>()? {
					match field.as_ref() {
						"id" => {
							if id.is_some() {
								return Err(de::Error::duplicate_field("id"));
							}
							let raw = map.next_value::<Cow<'_, str>>()?;
							id = Some(
								self.state
									.intern_identifier(raw.as_ref())
									.map_err(de::Error::custom)?,
							);
						}
						"args" => {
							if args.is_some() {
								return Err(de::Error::duplicate_field("args"));
							}
							args = Some(
								map.next_value_seed(self.state.seed::<ParseAnnotationArguments>())?,
							);
						}
						field => {
							return Err(de::Error::unknown_field(field, &["id", "args"]));
						}
					}
				}

				Ok(Annotation::Call(AnnotationCall {
					id: id.ok_or_else(|| de::Error::missing_field("id"))?,
					args: args.ok_or_else(|| de::Error::missing_field("args"))?,
				}))
			}
		}

		deserializer.deserialize_any(AnnotationVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de>
	for Seed<'_, Identifier, F, ParseAnnotationArgument>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = AnnotationArgument<Identifier>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct AnnotationArgumentVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for AnnotationArgumentVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = AnnotationArgument<Identifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("an annotation literal or an annotation literal array")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(value) =
					seq.next_element_seed(self.state.seed::<ParseAnnotationLiteral>())?
				{
					values.push(value);
				}
				Ok(AnnotationArgument::Array(values))
			}

			fn visit_i64<E2>(self, value: i64) -> Result<Self::Value, E2> {
				Ok(AnnotationArgument::Literal(AnnotationLiteral::Int(value)))
			}

			fn visit_u64<E2>(self, value: u64) -> Result<Self::Value, E2>
			where
				E2: de::Error,
			{
				let value = i64::try_from(value)
					.map_err(|_| E2::custom("integer literal is out of range"))?;
				Ok(AnnotationArgument::Literal(AnnotationLiteral::Int(value)))
			}

			fn visit_f64<E2>(self, value: f64) -> Result<Self::Value, E2> {
				Ok(AnnotationArgument::Literal(AnnotationLiteral::Float(value)))
			}

			fn visit_bool<E2>(self, value: bool) -> Result<Self::Value, E2> {
				Ok(AnnotationArgument::Literal(AnnotationLiteral::Bool(value)))
			}

			fn visit_borrowed_str<E2>(self, value: &'de str) -> Result<Self::Value, E2> {
				Ok(AnnotationArgument::Literal(AnnotationLiteral::Reference(
					self.state.names.intern(value),
				)))
			}

			fn visit_str<E2>(self, value: &str) -> Result<Self::Value, E2> {
				Ok(AnnotationArgument::Literal(AnnotationLiteral::Reference(
					self.state.names.intern(value),
				)))
			}

			fn visit_string<E2>(self, value: String) -> Result<Self::Value, E2> {
				Ok(AnnotationArgument::Literal(AnnotationLiteral::Reference(
					self.state.names.intern(&value),
				)))
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let field = map
					.next_key::<Cow<'_, str>>()?
					.ok_or_else(|| de::Error::custom("expected an annotation literal object"))?;
				Ok(AnnotationArgument::Literal(deserialize_annotation_literal(
					field, &mut map, self.state,
				)?))
			}
		}

		deserializer.deserialize_any(AnnotationArgumentVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de>
	for Seed<'_, Identifier, F, ParseAnnotationArguments>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = Vec<AnnotationArgument<Identifier>>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct AnnotationArgumentsVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for AnnotationArgumentsVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = Vec<AnnotationArgument<Identifier>>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a JSON array of annotation arguments")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut args = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(arg) =
					seq.next_element_seed(self.state.seed::<ParseAnnotationArgument>())?
				{
					args.push(arg);
				}
				Ok(args)
			}
		}

		deserializer.deserialize_seq(AnnotationArgumentsVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseAnnotationLiteral>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = AnnotationLiteral<Identifier>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct AnnotationLiteralVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for AnnotationLiteralVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = AnnotationLiteral<Identifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("an annotation literal")
			}

			fn visit_i64<E2>(self, value: i64) -> Result<Self::Value, E2> {
				Ok(AnnotationLiteral::Int(value))
			}

			fn visit_u64<E2>(self, value: u64) -> Result<Self::Value, E2>
			where
				E2: de::Error,
			{
				let value = i64::try_from(value)
					.map_err(|_| E2::custom("integer literal is out of range"))?;
				Ok(AnnotationLiteral::Int(value))
			}

			fn visit_f64<E2>(self, value: f64) -> Result<Self::Value, E2> {
				Ok(AnnotationLiteral::Float(value))
			}

			fn visit_bool<E2>(self, value: bool) -> Result<Self::Value, E2> {
				Ok(AnnotationLiteral::Bool(value))
			}

			fn visit_borrowed_str<E2>(self, value: &'de str) -> Result<Self::Value, E2> {
				Ok(AnnotationLiteral::Reference(self.state.names.intern(value)))
			}

			fn visit_str<E2>(self, value: &str) -> Result<Self::Value, E2> {
				Ok(AnnotationLiteral::Reference(self.state.names.intern(value)))
			}

			fn visit_string<E2>(self, value: String) -> Result<Self::Value, E2> {
				Ok(AnnotationLiteral::Reference(
					self.state.names.intern(&value),
				))
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let field = map
					.next_key::<Cow<'_, str>>()?
					.ok_or_else(|| de::Error::custom("expected an annotation literal object"))?;
				deserialize_annotation_literal(field, &mut map, self.state)
			}
		}

		deserializer.deserialize_any(AnnotationLiteralVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseAnnotations>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = Vec<Annotation<Identifier>>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct AnnotationsVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for AnnotationsVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = Vec<Annotation<Identifier>>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a JSON array of annotations")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut annotations = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(annotation) =
					seq.next_element_seed(self.state.seed::<ParseAnnotation>())?
				{
					annotations.push(annotation);
				}
				Ok(annotations)
			}
		}

		deserializer.deserialize_seq(AnnotationsVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseArgument>
where
	Identifier: Clone,
{
	type Value = Argument;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct ArgumentVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F> Visitor<'de> for ArgumentVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
		{
			type Value = Argument;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a literal or an array of literals")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(value) = seq.next_element_seed(self.state.seed::<ParseLiteral>())? {
					values.push(value);
				}
				Ok(Argument::Array(values))
			}

			fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
				Ok(Argument::Literal(Literal::Int(value)))
			}

			fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
			where
				E: de::Error,
			{
				let value = i64::try_from(value)
					.map_err(|_| E::custom("integer literal is out of range"))?;
				Ok(Argument::Literal(Literal::Int(value)))
			}

			fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
				Ok(Argument::Literal(Literal::Float(value)))
			}

			fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
				Ok(Argument::Literal(Literal::Bool(value)))
			}

			fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E> {
				Ok(Argument::Literal(Literal::Reference(
					self.state.names.intern(value),
				)))
			}

			fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
				Ok(Argument::Literal(Literal::Reference(
					self.state.names.intern(value),
				)))
			}

			fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
				Ok(Argument::Literal(Literal::Reference(
					self.state.names.intern(&value),
				)))
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let field = map
					.next_key::<Cow<'_, str>>()?
					.ok_or_else(|| de::Error::custom("expected a string or set literal object"))?;
				match field.as_ref() {
					"set" => {
						let ranges = map.next_value::<Vec<(NumberValue, NumberValue)>>()?;
						ensure_object_finished(&mut map)?;
						Ok(Argument::Literal(
							match ranges.try_into().map_err(de::Error::custom)? {
								NumericSet::Int(ranges) => Literal::IntSet(ranges),
								NumericSet::Float(ranges) => Literal::FloatSet(ranges),
							},
						))
					}
					"string" => {
						let string = map.next_value::<String>()?;
						ensure_object_finished(&mut map)?;
						Ok(Argument::Literal(Literal::String(string)))
					}
					_ => Err(de::Error::custom("expected a string or set literal object")),
				}
			}
		}

		deserializer.deserialize_any(ArgumentVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseArguments>
where
	Identifier: Clone,
{
	type Value = Vec<Argument>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct ArgumentsVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F> Visitor<'de> for ArgumentsVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
		{
			type Value = Vec<Argument>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("an array of arguments")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut args = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(arg) = seq.next_element_seed(self.state.seed::<ParseArgument>())? {
					args.push(arg);
				}
				Ok(args)
			}
		}

		deserializer.deserialize_seq(ArgumentsVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseArray>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = Array<Identifier>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct ArrayVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for ArrayVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = Array<Identifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a FlatZinc array object")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut contents = None;
				let mut ann = Vec::new();
				let mut defined = false;
				let mut introduced = false;

				while let Some(field) = map.next_key::<Cow<'_, str>>()? {
					match field.as_ref() {
						"a" => {
							if contents.is_some() {
								return Err(de::Error::duplicate_field("a"));
							}
							contents =
								Some(map.next_value_seed(self.state.seed::<ParseLiterals>())?);
						}
						"ann" => {
							ann = map.next_value_seed(self.state.seed::<ParseAnnotations>())?;
						}
						"defined" => defined = map.next_value()?,
						"introduced" => introduced = map.next_value()?,
						field => {
							return Err(de::Error::unknown_field(
								field,
								&["a", "ann", "defined", "introduced"],
							));
						}
					}
				}

				Ok(Array {
					contents: contents.ok_or_else(|| de::Error::missing_field("a"))?,
					ann,
					defined,
					introduced,
				})
			}
		}

		deserializer.deserialize_map(ArrayVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseArrays>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = ();

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct ArraysVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for ArraysVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = ();

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a JSON object containing FlatZinc arrays")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				while let Some(name) = map.next_key::<Cow<'_, str>>()? {
					let name_id = self.state.names.intern(name.as_ref());
					let array = map.next_value_seed(self.state.seed::<ParseArray>())?;
					self.state
						.define_array(name_id, array)
						.map_err(de::Error::custom)?;
				}
				Ok(())
			}
		}

		deserializer.deserialize_map(ArraysVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseConstraint>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = Constraint<Identifier>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct ConstraintVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for ConstraintVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = Constraint<Identifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a FlatZinc constraint object")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut id = None;
				let mut args = None;
				let mut defines = None;
				let mut ann = Vec::new();

				while let Some(field) = map.next_key::<Cow<'_, str>>()? {
					match field.as_ref() {
						"id" => {
							if id.is_some() {
								return Err(de::Error::duplicate_field("id"));
							}
							let raw = map.next_value::<Cow<'_, str>>()?;
							id = Some(
								self.state
									.intern_identifier(raw.as_ref())
									.map_err(de::Error::custom)?,
							);
						}
						"args" => {
							if args.is_some() {
								return Err(de::Error::duplicate_field("args"));
							}
							args = Some(map.next_value_seed(self.state.seed::<ParseArguments>())?);
						}
						"defines" => {
							let name = map.next_value::<Cow<'_, str>>()?;
							defines = Some(self.state.names.intern(name.as_ref()));
						}
						"ann" => {
							ann = map.next_value_seed(self.state.seed::<ParseAnnotations>())?;
						}
						field => {
							return Err(de::Error::unknown_field(
								field,
								&["id", "args", "defines", "ann"],
							));
						}
					}
				}

				Ok(Constraint {
					id: id.ok_or_else(|| de::Error::missing_field("id"))?,
					args: args.ok_or_else(|| de::Error::missing_field("args"))?,
					defines,
					ann,
				})
			}
		}

		deserializer.deserialize_map(ConstraintVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseConstraints>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = Vec<Constraint<Identifier>>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct ConstraintsVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for ConstraintsVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = Vec<Constraint<Identifier>>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a JSON array of FlatZinc constraints")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut constraints = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(constraint) =
					seq.next_element_seed(self.state.seed::<ParseConstraint>())?
				{
					constraints.push(constraint);
				}
				Ok(constraints)
			}
		}

		deserializer.deserialize_seq(ConstraintsVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseLiteral> {
	type Value = Literal;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct LiteralVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F> Visitor<'de> for LiteralVisitor<'_, Identifier, F> {
			type Value = Literal;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a FlatZinc literal")
			}

			fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
				Ok(Literal::Int(value))
			}

			fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
			where
				E: de::Error,
			{
				let value = i64::try_from(value)
					.map_err(|_| E::custom("integer literal is out of range"))?;
				Ok(Literal::Int(value))
			}

			fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
				Ok(Literal::Float(value))
			}

			fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
				Ok(Literal::Bool(value))
			}

			fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E> {
				Ok(Literal::Reference(self.state.names.intern(value)))
			}

			fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
				Ok(Literal::Reference(self.state.names.intern(value)))
			}

			fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
				Ok(Literal::Reference(self.state.names.intern(&value)))
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let field = map
					.next_key::<Cow<'_, str>>()?
					.ok_or_else(|| de::Error::custom("expected a string or set literal object"))?;
				match field.as_ref() {
					"set" => {
						let ranges = map.next_value::<Vec<(NumberValue, NumberValue)>>()?;
						ensure_object_finished(&mut map)?;
						match ranges.try_into().map_err(de::Error::custom)? {
							NumericSet::Int(ranges) => Ok(Literal::IntSet(ranges)),
							NumericSet::Float(ranges) => Ok(Literal::FloatSet(ranges)),
						}
					}
					"string" => {
						let string = map.next_value::<String>()?;
						ensure_object_finished(&mut map)?;
						Ok(Literal::String(string))
					}
					_ => Err(de::Error::custom("expected a string or set literal object")),
				}
			}
		}

		deserializer.deserialize_any(LiteralVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseLiterals> {
	type Value = Vec<Literal>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct LiteralsVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F> Visitor<'de> for LiteralsVisitor<'_, Identifier, F> {
			type Value = Vec<Literal>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a JSON array of literals")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(value) = seq.next_element_seed(self.state.seed::<ParseLiteral>())? {
					values.push(value);
				}
				Ok(values)
			}
		}

		deserializer.deserialize_seq(LiteralsVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseOutput> {
	type Value = Vec<NameId>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct OutputVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F> Visitor<'de> for OutputVisitor<'_, Identifier, F> {
			type Value = Vec<NameId>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a JSON array of model names")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut output = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(name) = seq.next_element::<Cow<'_, str>>()? {
					output.push(self.state.names.intern(name.as_ref()));
				}
				Ok(output)
			}
		}

		deserializer.deserialize_seq(OutputVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseSolve>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = SolveObjective<Identifier>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct SolveVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for SolveVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = SolveObjective<Identifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a FlatZinc solve object")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut method = None;
				let mut objective = None;
				let mut ann = Vec::new();

				while let Some(field) = map.next_key::<Cow<'_, str>>()? {
					match field.as_ref() {
						"method" => {
							if method.is_some() {
								return Err(de::Error::duplicate_field("method"));
							}
							method = Some(map.next_value::<Cow<'_, str>>()?);
						}
						"objective" => {
							if objective.is_some() {
								return Err(de::Error::duplicate_field("objective"));
							}
							objective =
								Some(map.next_value_seed(self.state.seed::<ParseLiteral>())?);
						}
						"ann" => {
							ann = map.next_value_seed(self.state.seed::<ParseAnnotations>())?;
						}
						field => {
							return Err(de::Error::unknown_field(
								field,
								&["method", "objective", "ann"],
							));
						}
					}
				}

				let method = method.ok_or_else(|| de::Error::missing_field("method"))?;
				let method = match (method.as_ref(), objective) {
					("satisfy", None) => Method::Satisfy,
					("satisfy", Some(_)) => {
						return Err(de::Error::custom(
							"satisfy solve items cannot have an objective",
						));
					}
					("minimize", Some(objective)) => Method::Minimize(objective),
					("minimize", None) => {
						return Err(de::Error::custom(
							"minimize solve items require an objective",
						));
					}
					("maximize", Some(objective)) => Method::Maximize(objective),
					("maximize", None) => {
						return Err(de::Error::custom(
							"maximize solve items require an objective",
						));
					}
					(method, _) => {
						return Err(de::Error::custom(format!(
							"unknown solve method `{method}`",
						)));
					}
				};

				Ok(SolveObjective { method, ann })
			}
		}

		deserializer.deserialize_map(SolveVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseVariable>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = Variable<Identifier>;

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct VariableVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for VariableVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = Variable<Identifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a FlatZinc variable object")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut ty = None;
				let mut domain = None;
				let mut ann = Vec::new();
				let mut defined = false;
				let mut introduced = false;

				while let Some(field) = map.next_key::<Cow<'_, str>>()? {
					match field.as_ref() {
						"type" => {
							if ty.is_some() {
								return Err(de::Error::duplicate_field("type"));
							}
							ty = Some(map.next_value::<BaseType>()?);
						}
						"domain" => {
							if domain.is_some() {
								return Err(de::Error::duplicate_field("domain"));
							}
							domain = Some(map.next_value::<VariableDomain>()?);
						}
						"ann" => {
							ann = map.next_value_seed(self.state.seed::<ParseAnnotations>())?;
						}
						"defined" => defined = map.next_value()?,
						"introduced" => introduced = map.next_value()?,
						field => {
							return Err(de::Error::unknown_field(
								field,
								&["type", "domain", "ann", "defined", "introduced"],
							));
						}
					}
				}

				Ok(Variable {
					ty: ty
						.ok_or_else(|| de::Error::missing_field("type"))?
						.into_type(domain)
						.map_err(de::Error::custom)?,
					ann,
					defined,
					introduced,
				})
			}
		}

		deserializer.deserialize_map(VariableVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}

impl<'de, Identifier, F, E> DeserializeSeed<'de> for Seed<'_, Identifier, F, ParseVariables>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	type Value = ();

	fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct VariablesVisitor<'a, Identifier, F> {
			state: &'a mut ParserState<Identifier, F>,
			marker: PhantomData<Identifier>,
		}

		impl<'de, Identifier, F, E> Visitor<'de> for VariablesVisitor<'_, Identifier, F>
		where
			Identifier: Clone,
			F: FnMut(&str) -> Result<Identifier, E>,
			E: Display,
		{
			type Value = ();

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a JSON object containing FlatZinc variables")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				while let Some(name) = map.next_key::<Cow<'_, str>>()? {
					let name_id = self.state.names.intern(name.as_ref());
					let variable = map.next_value_seed(self.state.seed::<ParseVariable>())?;
					self.state
						.define_variable(name_id, variable)
						.map_err(de::Error::custom)?;
				}
				Ok(())
			}
		}

		deserializer.deserialize_map(VariablesVisitor {
			state: self.state,
			marker: PhantomData,
		})
	}
}
