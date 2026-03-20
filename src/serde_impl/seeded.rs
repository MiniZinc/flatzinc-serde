//! Seeded FlatZinc deserialization with alias resolution and custom interning.

use std::{
	borrow::Cow,
	collections::HashMap,
	fmt::{self, Debug, Display, Formatter},
	marker::PhantomData,
};

use serde::{
	Deserialize, Deserializer,
	de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};

use super::{BaseType, VariableDomain, deserialize_key_value_object};
use crate::{
	Annotation, AnnotationArgument, AnnotationCall, AnnotationLiteral, Argument, Array, Constraint,
	FlatZinc, Literal, Method, SolveObjective, Variable,
};

/// Wrapper used by seeded deserialization to collect fully converted arrays.
struct ArraysField<I>(Vec<(I, Array<I>)>);

/// Deferred representation of the `arrays` field before identifiers have been
/// interned and aliases resolved.
#[derive(Deserialize)]
struct ArraysFieldCow<'input>(
	#[serde(borrow, deserialize_with = "deserialize_key_value_object")]
	Vec<(Cow<'input, str>, Array<Cow<'input, str>>)>,
);

/// Wrapper used by seeded deserialization to collect fully converted
/// constraints.
struct ConstraintsField<I>(Vec<Constraint<I>>);

/// Wrapper used by seeded deserialization to collect fully converted output
/// identifiers.
struct OutputField<I>(Vec<I>);

/// Mutable parser state shared across the seeded FlatZinc JSON visitor.
///
/// This state owns the caller-provided interner and caches resolved variable
/// aliases so later fields can be converted directly into their final
/// identifier type.
struct ParserState<'de, I, F> {
	/// Map from variable names to their fully resolved literal values.
	aliases: HashMap<Cow<'de, str>, Literal<I>>,
	/// Caller-provided identifier conversion or interning function.
	interner: F,
}

/// Generic seed used to deserialize individual FlatZinc fragments while
/// sharing a mutable [`ParserState`].
struct Seed<'a, 'de, I, F, T> {
	/// Shared parser state used by all nested visitors and conversions.
	parser: &'a mut ParserState<'de, I, F>,
	/// Marker for the concrete target type this seed should deserialize.
	target: PhantomData<T>,
}

/// Internal variable representation that preserves the JSON `rhs` field during
/// deserialization.
///
/// The public [`Variable`] type does not store a right-hand side. Instead, the
/// seeded deserializer uses this representation transiently so aliases can be
/// resolved before constructing the final model.
#[derive(Clone, PartialEq, Debug)]
pub(super) struct VariableValue<Identifier = String> {
	/// The type and optional domain of the variable.
	ty: crate::Type,
	/// Optional right-hand side stored in the JSON `"rhs"` field.
	pub(super) value: Option<Literal<Identifier>>,
	/// Variable annotations.
	ann: Vec<Annotation<Identifier>>,
	/// Whether the variable is solver-defined.
	defined: bool,
	/// Whether the variable was introduced during lowering.
	introduced: bool,
}

/// Deferred representation of the `variables` field before alias resolution
/// and identifier interning have been applied.
#[derive(Deserialize)]
struct VariablesField<'input>(
	#[serde(borrow, deserialize_with = "deserialize_key_value_object")]
	Vec<(Cow<'input, str>, VariableValue<Cow<'input, str>>)>,
);

/// Deserialize a FlatZinc JSON value using a custom identifier interner while
/// resolving variable aliases eagerly.
pub(crate) fn deserialize_flatzinc_with_interner<'de, D, I, VM, AM, F, E>(
	deserializer: D,
	interner: F,
) -> Result<FlatZinc<I, VM, AM>, D::Error>
where
	D: Deserializer<'de>,
	I: Clone,
	VM: FromIterator<(I, Variable<I>)>,
	AM: FromIterator<(I, Array<I>)>,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	let mut parser = ParserState {
		aliases: HashMap::new(),
		interner,
	};
	Seed {
		parser: &mut parser,
		target: PhantomData::<FlatZinc<I, VM, AM>>,
	}
	.deserialize(deserializer)
}

impl<'de, I, F, E> ParserState<'de, I, F>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	/// Convert an annotation from its borrowed intermediate representation into
	/// the final identifier type.
	fn convert_annotation<DE: serde::de::Error>(
		&mut self,
		annotation: Annotation<Cow<'de, str>>,
	) -> Result<Annotation<I>, DE> {
		match annotation {
			Annotation::Atom(ident) => Ok(Annotation::Atom(self.intern::<DE>(ident)?)),
			Annotation::Call(call) => Ok(Annotation::Call(self.convert_annotation_call(call)?)),
		}
	}

	/// Convert an annotation argument from its borrowed intermediate
	/// representation into the final identifier type.
	fn convert_annotation_argument<DE: serde::de::Error>(
		&mut self,
		arg: AnnotationArgument<Cow<'de, str>>,
	) -> Result<AnnotationArgument<I>, DE> {
		match arg {
			AnnotationArgument::Array(values) => Ok(AnnotationArgument::Array(
				values
					.into_iter()
					.map(|value| self.convert_annotation_literal(value))
					.collect::<Result<_, _>>()?,
			)),
			AnnotationArgument::Literal(value) => Ok(AnnotationArgument::Literal(
				self.convert_annotation_literal(value)?,
			)),
		}
	}

	/// Convert an annotation call from its borrowed intermediate representation
	/// into the final identifier type.
	fn convert_annotation_call<DE: serde::de::Error>(
		&mut self,
		call: AnnotationCall<Cow<'de, str>>,
	) -> Result<AnnotationCall<I>, DE> {
		Ok(AnnotationCall {
			id: self.intern::<DE>(call.id)?,
			args: call
				.args
				.into_iter()
				.map(|arg| self.convert_annotation_argument(arg))
				.collect::<Result<_, _>>()?,
		})
	}

	/// Convert an annotation literal from its borrowed intermediate
	/// representation into the final identifier type.
	fn convert_annotation_literal<DE: serde::de::Error>(
		&mut self,
		literal: AnnotationLiteral<Cow<'de, str>>,
	) -> Result<AnnotationLiteral<I>, DE> {
		match literal {
			AnnotationLiteral::BaseLiteral(literal) => Ok(AnnotationLiteral::BaseLiteral(
				self.convert_literal(literal)?,
			)),
			AnnotationLiteral::Annotation(call) => Ok(AnnotationLiteral::Annotation(
				self.convert_annotation_call(call)?,
			)),
		}
	}

	/// Convert a constraint argument from its borrowed intermediate
	/// representation into the final identifier type.
	fn convert_argument<DE: serde::de::Error>(
		&mut self,
		arg: Argument<Cow<'de, str>>,
	) -> Result<Argument<I>, DE> {
		match arg {
			Argument::Array(values) => Ok(Argument::Array(
				values
					.into_iter()
					.map(|value| self.convert_literal(value))
					.collect::<Result<_, _>>()?,
			)),
			Argument::Literal(value) => Ok(Argument::Literal(self.convert_literal(value)?)),
		}
	}

	/// Convert an array definition from borrowed identifiers into the final
	/// identifier type while resolving aliases in its contents.
	fn convert_array<DE: serde::de::Error>(
		&mut self,
		array: Array<Cow<'de, str>>,
	) -> Result<Array<I>, DE> {
		Ok(Array {
			contents: array
				.contents
				.into_iter()
				.map(|value| self.convert_literal(value))
				.collect::<Result<_, _>>()?,
			ann: array
				.ann
				.into_iter()
				.map(|ann| self.convert_annotation(ann))
				.collect::<Result<_, _>>()?,
			defined: array.defined,
			introduced: array.introduced,
		})
	}

	/// Convert a constraint from borrowed identifiers into the final identifier
	/// type while resolving aliases in its arguments.
	fn convert_constraint<DE: serde::de::Error>(
		&mut self,
		constraint: Constraint<Cow<'de, str>>,
	) -> Result<Constraint<I>, DE> {
		Ok(Constraint {
			id: self.intern::<DE>(constraint.id)?,
			args: constraint
				.args
				.into_iter()
				.map(|arg| self.convert_argument(arg))
				.collect::<Result<_, _>>()?,
			defines: constraint
				.defines
				.map(|ident| self.intern::<DE>(ident))
				.transpose()?,
			ann: constraint
				.ann
				.into_iter()
				.map(|ann| self.convert_annotation(ann))
				.collect::<Result<_, _>>()?,
		})
	}

	/// Convert a literal from borrowed identifiers into the final identifier
	/// type, resolving aliases when the literal refers to an aliased variable.
	fn convert_literal<DE: serde::de::Error>(
		&mut self,
		literal: Literal<Cow<'de, str>>,
	) -> Result<Literal<I>, DE> {
		match literal {
			Literal::Identifier(ident) => {
				if let Some(literal) = self.aliases.get(&ident).cloned() {
					Ok(literal)
				} else {
					Ok(Literal::Identifier(self.intern::<DE>(ident)?))
				}
			}
			Literal::Int(i) => Ok(Literal::Int(i)),
			Literal::Float(f) => Ok(Literal::Float(f)),
			Literal::Bool(b) => Ok(Literal::Bool(b)),
			Literal::IntSet(r) => Ok(Literal::IntSet(r)),
			Literal::FloatSet(r) => Ok(Literal::FloatSet(r)),
			Literal::String(s) => Ok(Literal::String(s)),
		}
	}

	/// Convert a solve item from borrowed identifiers into the final identifier
	/// type while resolving aliases inside objectives and annotations.
	fn convert_solve<DE: serde::de::Error>(
		&mut self,
		solve: SolveObjective<Cow<'de, str>>,
	) -> Result<SolveObjective<I>, DE> {
		let method = match solve.method {
			Method::Satisfy => Method::Satisfy,
			Method::Minimize(objective) => Method::Minimize(self.convert_literal(objective)?),
			Method::Maximize(objective) => Method::Maximize(self.convert_literal(objective)?),
		};
		Ok(SolveObjective {
			method,
			ann: solve
				.ann
				.into_iter()
				.map(|ann| self.convert_annotation(ann))
				.collect::<Result<_, _>>()?,
		})
	}

	/// Parse the `variables` field, populate the alias cache, and return only
	/// the non-alias variables in the final identifier type.
	fn parse_variables<DE: serde::de::Error>(
		&mut self,
		raw_variables: Vec<(Cow<'de, str>, VariableValue<Cow<'de, str>>)>,
	) -> Result<Vec<(I, Variable<I>)>, DE> {
		let raw_aliases: HashMap<Cow<'de, str>, Literal<Cow<'de, str>>> = raw_variables
			.iter()
			.filter_map(|(name, variable)| {
				variable.value.clone().map(|value| (name.clone(), value))
			})
			.collect();

		for (name, literal) in &raw_aliases {
			let resolved =
				self.resolve_alias_literal(&raw_aliases, &mut vec![name.clone()], literal)?;
			let _ = self.aliases.insert(name.clone(), resolved);
		}

		raw_variables
			.into_iter()
			.filter(|(_, variable)| variable.value.is_none())
			.map(|(name, variable)| {
				let ident = self.intern::<DE>(name)?;
				let variable = Variable {
					ty: variable.ty,
					ann: variable
						.ann
						.into_iter()
						.map(|ann| self.convert_annotation(ann))
						.collect::<Result<_, DE>>()?,
					defined: variable.defined,
					introduced: variable.introduced,
				};
				Ok((ident, variable))
			})
			.collect()
	}

	/// Resolve a variable alias chain to its final literal value.
	///
	/// The function walks aliases iteratively, detects cycles, and memoizes any
	/// intermediate aliases it encounters in [`ParserState::aliases`].
	fn resolve_alias_literal<DE: serde::de::Error>(
		&mut self,
		raw_aliases: &HashMap<Cow<'de, str>, Literal<Cow<'de, str>>>,
		resolving: &mut Vec<Cow<'de, str>>,
		literal: &Literal<Cow<'de, str>>,
	) -> Result<Literal<I>, DE> {
		let mut current = literal;
		let resolved = loop {
			match current {
				Literal::Identifier(ident) => {
					if let Some(resolved) = self.aliases.get(ident.as_ref()).cloned() {
						break resolved;
					}
					if let Some(alias) = raw_aliases.get(ident.as_ref()) {
						if resolving.iter().any(|name| name == ident) {
							return Err(DE::custom(format!(
								"cyclic variable alias involving `{ident}`"
							)));
						}
						resolving.push(ident.clone());
						current = alias;
					} else {
						break Literal::Identifier(self.intern::<DE>(ident.clone())?);
					}
				}
				Literal::Int(i) => break Literal::Int(*i),
				Literal::Float(f) => break Literal::Float(*f),
				Literal::Bool(b) => break Literal::Bool(*b),
				Literal::IntSet(r) => break Literal::IntSet(r.clone()),
				Literal::FloatSet(r) => break Literal::FloatSet(r.clone()),
				Literal::String(s) => break Literal::String(s.clone()),
			}
		};

		for ident in resolving.iter().skip(1) {
			let _ = self.aliases.insert(ident.clone(), resolved.clone());
		}
		Ok(resolved)
	}
}

impl<'de, I, F, E> ParserState<'de, I, F>
where
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	/// Convert a borrowed or owned identifier string into the caller's
	/// identifier type.
	fn intern<DE: serde::de::Error>(&mut self, ident: Cow<'de, str>) -> Result<I, DE> {
		(self.interner)(ident).map_err(|err| DE::custom(err.to_string()))
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, Annotation<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = Annotation<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let ann = Annotation::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_annotation(ann)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, AnnotationArgument<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = AnnotationArgument<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let arg = AnnotationArgument::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_annotation_argument(arg)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, AnnotationCall<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = AnnotationCall<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let call = AnnotationCall::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_annotation_call(call)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, AnnotationLiteral<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = AnnotationLiteral<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let literal = AnnotationLiteral::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_annotation_literal(literal)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, Argument<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = Argument<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let arg = Argument::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_argument(arg)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, Array<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = Array<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let array = Array::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_array(array)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, ArraysField<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = ArraysField<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		struct ArraysVisitor<'a, 'de, I, F> {
			parser: &'a mut ParserState<'de, I, F>,
		}

		impl<'a, 'de, I, F, E> Visitor<'de> for ArraysVisitor<'a, 'de, I, F>
		where
			I: Clone,
			F: FnMut(Cow<'de, str>) -> Result<I, E>,
			E: Display,
		{
			type Value = ArraysField<I>;

			fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
				formatter.write_str("a JSON object of arrays")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut arrays = Vec::new();
				while let Some(name) = map.next_key_seed(Seed {
					parser: self.parser,
					target: PhantomData::<I>,
				})? {
					let array = map.next_value_seed(Seed {
						parser: self.parser,
						target: PhantomData::<Array<I>>,
					})?;
					arrays.push((name, array));
				}
				Ok(ArraysField(arrays))
			}
		}

		deserializer.deserialize_map(ArraysVisitor {
			parser: self.parser,
		})
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, Constraint<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = Constraint<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let constraint = Constraint::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_constraint(constraint)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, ConstraintsField<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = ConstraintsField<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		struct ConstraintsVisitor<'a, 'de, I, F> {
			parser: &'a mut ParserState<'de, I, F>,
		}

		impl<'a, 'de, I, F, E> Visitor<'de> for ConstraintsVisitor<'a, 'de, I, F>
		where
			I: Clone,
			F: FnMut(Cow<'de, str>) -> Result<I, E>,
			E: Display,
		{
			type Value = ConstraintsField<I>;

			fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
				formatter.write_str("a JSON array of constraints")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut constraints = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(constraint) = seq.next_element_seed(Seed {
					parser: self.parser,
					target: PhantomData::<Constraint<I>>,
				})? {
					constraints.push(constraint);
				}
				Ok(ConstraintsField(constraints))
			}
		}

		deserializer.deserialize_seq(ConstraintsVisitor {
			parser: self.parser,
		})
	}
}

impl<'a, 'de, I, F, VM, AM, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, FlatZinc<I, VM, AM>>
where
	I: Clone,
	VM: FromIterator<(I, Variable<I>)>,
	AM: FromIterator<(I, Array<I>)>,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = FlatZinc<I, VM, AM>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		#[derive(Deserialize)]
		#[serde(field_identifier, rename_all = "lowercase")]
		enum Field {
			Variables,
			Arrays,
			Constraints,
			Output,
			Solve,
			Version,
			#[serde(other)]
			Other,
		}

		struct FlatZincVisitor<'a, 'de, I, F, VM, AM> {
			parser: &'a mut ParserState<'de, I, F>,
			target: PhantomData<(VM, AM)>,
		}

		impl<'a, 'de, I, F, VM, AM, E> Visitor<'de> for FlatZincVisitor<'a, 'de, I, F, VM, AM>
		where
			I: Clone,
			VM: FromIterator<(I, Variable<I>)>,
			AM: FromIterator<(I, Array<I>)>,
			F: FnMut(Cow<'de, str>) -> Result<I, E>,
			E: Display,
		{
			type Value = FlatZinc<I, VM, AM>;

			fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
				formatter.write_str("a FlatZinc JSON object")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut variables = None;
				let mut arrays = None;
				let mut constraints = None;
				let mut output = None;
				let mut solve = None;
				let mut version = None;

				let mut pending_arrays: Option<ArraysFieldCow<'de>> = None;
				let mut pending_constraints: Option<Vec<Constraint<Cow<'de, str>>>> = None;
				let mut pending_output: Option<Vec<Cow<'de, str>>> = None;
				let mut pending_solve: Option<SolveObjective<Cow<'de, str>>> = None;

				while let Some(field) = map.next_key()? {
					match field {
						Field::Variables => {
							if variables.is_some() {
								return Err(serde::de::Error::duplicate_field("variables"));
							}
							let VariablesField(raw_variables) = map.next_value()?;
							variables =
								Some(self.parser.parse_variables::<A::Error>(raw_variables)?);

							if let Some(ArraysFieldCow(raw_arrays)) = pending_arrays.take() {
								arrays = Some(
									raw_arrays
										.into_iter()
										.map(|(name, array)| {
											Ok((
												self.parser.intern::<A::Error>(name)?,
												self.parser.convert_array::<A::Error>(array)?,
											))
										})
										.collect::<Result<AM, A::Error>>()?,
								);
							}
							if let Some(raw_constraints) = pending_constraints.take() {
								constraints = Some(
									raw_constraints
										.into_iter()
										.map(|constraint| {
											self.parser.convert_constraint::<A::Error>(constraint)
										})
										.collect::<Result<Vec<_>, _>>()?,
								);
							}
							if let Some(raw_output) = pending_output.take() {
								output = Some(
									raw_output
										.into_iter()
										.map(|ident| self.parser.intern::<A::Error>(ident))
										.collect::<Result<Vec<_>, _>>()?,
								);
							}
							if let Some(raw_solve) = pending_solve.take() {
								solve = Some(self.parser.convert_solve::<A::Error>(raw_solve)?);
							}
						}
						Field::Arrays => {
							if arrays.is_some() || pending_arrays.is_some() {
								return Err(serde::de::Error::duplicate_field("arrays"));
							}
							if variables.is_some() {
								let ArraysField(raw_arrays) = map.next_value_seed(Seed {
									parser: self.parser,
									target: PhantomData::<ArraysField<I>>,
								})?;
								arrays = Some(raw_arrays.into_iter().collect());
							} else {
								pending_arrays = Some(map.next_value()?);
							}
						}
						Field::Constraints => {
							if constraints.is_some() || pending_constraints.is_some() {
								return Err(serde::de::Error::duplicate_field("constraints"));
							}
							if variables.is_some() {
								let ConstraintsField(raw_constraints) =
									map.next_value_seed(Seed {
										parser: self.parser,
										target: PhantomData::<ConstraintsField<I>>,
									})?;
								constraints = Some(raw_constraints);
							} else {
								pending_constraints = Some(map.next_value()?);
							}
						}
						Field::Output => {
							if output.is_some() || pending_output.is_some() {
								return Err(serde::de::Error::duplicate_field("output"));
							}
							if variables.is_some() {
								let OutputField(raw_output) = map.next_value_seed(Seed {
									parser: self.parser,
									target: PhantomData::<OutputField<I>>,
								})?;
								output = Some(raw_output);
							} else {
								pending_output = Some(map.next_value()?);
							}
						}
						Field::Solve => {
							if solve.is_some() || pending_solve.is_some() {
								return Err(serde::de::Error::duplicate_field("solve"));
							}
							if variables.is_some() {
								solve = Some(map.next_value_seed(Seed {
									parser: self.parser,
									target: PhantomData::<SolveObjective<I>>,
								})?);
							} else {
								pending_solve = Some(map.next_value()?);
							}
						}
						Field::Version => {
							if version.is_some() {
								return Err(serde::de::Error::duplicate_field("version"));
							}
							version = Some(map.next_value()?);
						}
						Field::Other => {
							let _: IgnoredAny = map.next_value()?;
						}
					}
				}
				if arrays.is_none() {
					arrays = Some(
						pending_arrays
							.map(|arrays| arrays.0)
							.unwrap_or_default()
							.into_iter()
							.map(|(name, array)| {
								Ok((
									self.parser.intern::<A::Error>(name)?,
									self.parser.convert_array::<A::Error>(array)?,
								))
							})
							.collect::<Result<AM, A::Error>>()?,
					);
				}
				if constraints.is_none() {
					constraints = Some(
						pending_constraints
							.unwrap_or_default()
							.into_iter()
							.map(|constraint| {
								self.parser.convert_constraint::<A::Error>(constraint)
							})
							.collect::<Result<Vec<_>, _>>()?,
					);
				}
				if output.is_none() {
					output = Some(
						pending_output
							.unwrap_or_default()
							.into_iter()
							.map(|ident| self.parser.intern::<A::Error>(ident))
							.collect::<Result<Vec<_>, _>>()?,
					);
				}
				if solve.is_none() {
					return Err(serde::de::Error::missing_field("solve"));
				}

				Ok(FlatZinc {
					variables: variables.unwrap().into_iter().collect(),
					arrays: arrays.unwrap(),
					constraints: constraints.unwrap(),
					output: output.unwrap(),
					solve: solve.unwrap(),
					version: version.unwrap_or_default(),
				})
			}
		}

		deserializer.deserialize_map(FlatZincVisitor {
			parser: self.parser,
			target: PhantomData,
		})
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, I>
where
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = I;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let ident = Cow::<str>::deserialize(deserializer)?;
		self.parser.intern::<D::Error>(ident)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, Literal<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = Literal<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let literal = Literal::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_literal(literal)
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, OutputField<I>>
where
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = OutputField<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		struct OutputVisitor<'a, 'de, I, F> {
			parser: &'a mut ParserState<'de, I, F>,
		}

		impl<'a, 'de, I, F, E> Visitor<'de> for OutputVisitor<'a, 'de, I, F>
		where
			F: FnMut(Cow<'de, str>) -> Result<I, E>,
			E: Display,
		{
			type Value = OutputField<I>;

			fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
				formatter.write_str("a JSON array of identifiers")
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: SeqAccess<'de>,
			{
				let mut output = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(ident) = seq.next_element_seed(Seed {
					parser: self.parser,
					target: PhantomData::<I>,
				})? {
					output.push(ident);
				}
				Ok(OutputField(output))
			}
		}

		deserializer.deserialize_seq(OutputVisitor {
			parser: self.parser,
		})
	}
}

impl<'a, 'de, I, F, E> DeserializeSeed<'de> for Seed<'a, 'de, I, F, SolveObjective<I>>
where
	I: Clone,
	F: FnMut(Cow<'de, str>) -> Result<I, E>,
	E: Display,
{
	type Value = SolveObjective<I>;

	fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
		let solve = SolveObjective::<Cow<'de, str>>::deserialize(deserializer)?;
		self.parser.convert_solve(solve)
	}
}

impl<Identifier> VariableValue<Identifier> {
	/// Drop the internal right-hand side payload and convert to the public
	/// [`Variable`] representation.
	pub(super) fn into_variable(self) -> Variable<Identifier> {
		Variable {
			ty: self.ty,
			ann: self.ann,
			defined: self.defined,
			introduced: self.introduced,
		}
	}
}

impl<'de, Identifier: Deserialize<'de>> Deserialize<'de> for VariableValue<Identifier> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(rename = "variable")]
		#[serde(bound(deserialize = "Identifier: Deserialize<'de>"))]
		struct VariableRepr<Identifier> {
			#[serde(rename = "type")]
			ty: BaseType,
			#[serde(skip_serializing_if = "Option::is_none")]
			domain: Option<VariableDomain>,
			#[serde(rename = "rhs", skip_serializing_if = "Option::is_none")]
			value: Option<Literal<Identifier>>,
			#[serde(default, skip_serializing_if = "Vec::is_empty")]
			ann: Vec<Annotation<Identifier>>,
			#[serde(default)]
			defined: bool,
			#[serde(default)]
			introduced: bool,
		}

		let repr = VariableRepr::deserialize(deserializer)?;
		let ty = match (repr.ty, repr.domain) {
			(BaseType::Bool, None) => crate::Type::Bool,
			(BaseType::Bool, Some(_)) => {
				return Err(<D::Error as ::serde::de::Error>::custom(
					"bool variables cannot have a domain",
				));
			}
			(BaseType::Int, None) => crate::Type::Int(None),
			(BaseType::Int, Some(VariableDomain::Int(domain))) => crate::Type::Int(Some(domain)),
			(BaseType::Int, Some(VariableDomain::Float(_))) => {
				return Err(<D::Error as ::serde::de::Error>::custom(
					"int variables require an int domain",
				));
			}
			(BaseType::Float, None) => crate::Type::Float(None),
			(BaseType::Float, Some(VariableDomain::Float(domain))) => {
				crate::Type::Float(Some(domain))
			}
			(BaseType::Float, Some(VariableDomain::Int(_))) => {
				return Err(<D::Error as ::serde::de::Error>::custom(
					"float variables require a float domain",
				));
			}
			(BaseType::IntSet, None) => crate::Type::IntSet(None),
			(BaseType::IntSet, Some(VariableDomain::Int(domain))) => {
				crate::Type::IntSet(Some(domain))
			}
			(BaseType::IntSet, Some(VariableDomain::Float(_))) => {
				return Err(<D::Error as ::serde::de::Error>::custom(
					"set of int variables require an int domain",
				));
			}
		};

		Ok(VariableValue {
			ty,
			value: repr.value,
			ann: repr.ann,
			defined: repr.defined,
			introduced: repr.introduced,
		})
	}
}
