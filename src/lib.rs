//! Serialization of the FlatZinc data format
//!
//! FlatZinc is the language in which data and solver specific constraint models
//! are produced by the [MiniZinc](https://www.minizinc.org) compiler. This
//! crate implements the FlatZinc serialization format as described in the
//! [Interfacing Solvers to
//! FlatZinc](https://www.minizinc.org/doc-latest/en/fzn-spec.html#specification-of-flatzinc-json)
//! section of the MiniZinc reference manual. It supports both the JSON-based
//! FlatZinc representation, via [serde](https://serde.rs), and the older
//! textual `.fzn` format. For the JSON format, we suggest using
//! [`serde_json`](https://crates.io/crates/serde_json) with the specification
//! in this crate to parse the FlatZinc JSON files produced by the MiniZinc
//! compiler.
//!
//! # Feature Flags
//!
//! - `serde` (default): enables JSON serialization and deserialization support
//!   via the [`serde`](https://serde.rs) crate.
//! - `fzn`: enables parsing of the original `.fzn` text format via [`winnow`](https://crates.io/crates/winnow).
//!
//! # Getting Started
//!
//! For the default JSON-based workflow, install `flatzinc-serde` and
//! `serde_json` for your package:
//!
//! ```bash
//! cargo add flatzinc-serde serde_json
//! ```
//!
//! If you disable the default `serde` feature and only use the older textual
//! `.fzn` support, `serde_json` is not required.
//!
//! Once these dependencies have been installed to your crate, you can
//! deserialize a FlatZinc JSON file as follows:
//!
//! ```
//! # #[cfg(feature = "serde")] {
//! # use flatzinc_serde::FlatZinc;
//! # use std::{fs::File, io::BufReader, path::Path};
//! # let path = Path::new("./corpus/json/documentation_example.fzn.json");
//! // let path = Path::new("/lorem/ipsum/model.fzn.json");
//! let rdr = BufReader::new(File::open(path).unwrap());
//! let fzn: FlatZinc = serde_json::from_reader(rdr).unwrap();
//! // ... process FlatZinc ...
//! # }
//! ```
//!
//! When deserializing FlatZinc JSON, this crate rejects unknown fields on inner
//! FlatZinc objects such as variables, arrays, constraints, solve items, and
//! annotation-call objects. Unknown fields on the outer top-level wrapper
//! object are ignored to preserve some forward compatibility for envelope
//! metadata.
//!
//! The older textual `.fzn` format is also supported when the `fzn` feature is
//! enabled:
//!
//! ```
//! # #[cfg(feature = "fzn")] {
//! # use flatzinc_serde::FlatZinc;
//! # use std::{fs::File, io::BufReader, path::Path};
//! # let path = Path::new("./corpus/fzn/documentation_example.fzn");
//! // let path = Path::new("/lorem/ipsum/model.fzn");
//! let rdr = BufReader::new(File::open(path).unwrap());
//! let fzn: FlatZinc = FlatZinc::from_fzn(rdr).unwrap();
//! // ... process FlatZinc ...
//! # }
//! ```
//!
//! To serialize a FlatZinc JSON value, you can use the usual `serde_json`
//! APIs:
//!
//! ```
//! # #[cfg(feature = "serde")] {
//! # use flatzinc_serde::FlatZinc;
//! let fzn = FlatZinc::<String>::default();
//! // ... create  solver constraint model ...
//! let json_str = serde_json::to_string(&fzn).unwrap();
//! # }
//! ```
//! Note that `serde_json::to_writer`, using a buffered file writer, would be
//! preferred when writing larger FlatZinc files.
//!
//! To serialize a FlatZinc value to the older textual `.fzn` format, use its
//! [`Display`] implementation:
//!
//! ```
//! # use flatzinc_serde::FlatZinc;
//! let fzn = FlatZinc::<String>::default();
//! let fzn_text = fzn.to_string();
//! ```
//!
//! # Register your solver with MiniZinc
//!
//! If your goal is to deserialize FlatZinc to implement a MiniZinc solver, then
//! the next step is to register your solver executable with MiniZinc. This can
//! be done by creating a [MiniZinc Solver
//! Configuration](https://www.minizinc.org/doc-2.8.2/en/fzn-spec.html#solver-configuration-files)
//! (`.msc`) file, and adding it to a folder on the `MZN_SOLVER_PATH` or a
//! standardized path, like `~/.minizinc/solvers/`. A basic solver configuration
//! for a solver that accepts JSON input would look as follows:
//!
//! ```json
//! {
//!   "name" : "My Solver",
//!   "version": "0.0.1",
//!   "id": "my.organisation.mysolver",
//!   "inputType": "JSON",
//!   "executable": "../../../bin/fzn-my-solver",
//!   "mznlib": "../mysolver"
//!   "stdFlags": [],
//!   "extraFlags": []
//! }
//! ```
//!
//! Once you have placed your configuration file on the correct path, then you
//! solver will be listed by `minizinc --solvers`. Calling `minizinc --solver
//! mysolver model.mzn data.dzn`, assuming a valid MiniZinc instance, will
//! (after compilation) invoke the registered executable with a path of a
//! FlatZinc JSON file, and potentially any registered standard and extra flags
//! (e.g., `../../../bin/fzn-my-solver model.fzn.json`).

mod error;
#[cfg(feature = "fzn")]
mod fzn;
pub mod helpers;
#[cfg(any(feature = "fzn", feature = "serde"))]
mod intermediate;
#[cfg(feature = "serde")]
mod serde_impl;

use std::{
	borrow::Cow,
	cmp::Ordering,
	collections::HashSet,
	fmt::{Debug, Display},
	hash::{Hash, Hasher},
	sync::{Arc, RwLock},
};

pub use rangelist::RangeList;
#[cfg(feature = "serde")]
use serde::{Deserializer, Serialize};

pub use crate::error::{FznParseError, LinkError};
use crate::helpers::{ArcKey, FznRef, Immutable, Mutable};

/// Additional information provided in a standardized format for declarations,
/// constraints, or solve objectives
///
/// In MiniZinc annotations can both be added explicitly in the model, or can be
/// added during compilation process.
///
/// Note that annotations are generally defined either in the MiniZinc standard
/// library or in a solver's redefinition library. Solvers are encouraged to
/// rewrite annotations in their redefinitions library when required.
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[cfg_attr(
	feature = "serde",
	serde(bound(serialize = "Identifier: Serialize, Ref: FznRef"))
)]
#[derive(Clone, PartialEq, Debug)]
pub enum Annotation<Identifier = String, Ref: FznRef = Immutable> {
	/// Atom annotation (i.e., a single `Identifier`)
	Atom(Identifier),
	/// Call annotation
	Call(AnnotationCall<Identifier, Ref>),
}

/// An object depicting an annotation in the form of a call
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename = "annotation_call"))]
#[cfg_attr(
	feature = "serde",
	serde(bound(serialize = "Identifier: Serialize, Ref: FznRef"))
)]
#[derive(Clone, PartialEq, Debug)]
pub struct AnnotationCall<Identifier = String, Ref: FznRef = Immutable> {
	/// Identifier of the constraint predicate
	pub id: Identifier,
	/// Arguments of the constraint
	pub args: Vec<Argument<Identifier, Ref, AnnotationLiteral<Identifier, Ref>>>,
}

/// Literal values as arguments to [`AnnotationCall`]
///
/// These are the same as regular [`Literal`]s, except that they may
/// additionally be a nested [`Annotation`].
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[cfg_attr(
	feature = "serde",
	serde(bound(serialize = "Identifier: Serialize, Ref: FznRef"))
)]
#[derive(Clone, PartialEq, Debug)]
pub enum AnnotationLiteral<Identifier = String, Ref: FznRef = Immutable> {
	/// A regular literal value.
	Literal(Literal<Identifier, Ref>),
	/// An annotation object.
	Annotation(Annotation<Identifier, Ref>),
}

/// The argument type associated with [`Constraint`]
///
/// The literal type `L` is [`Literal`] for constraint arguments and
/// [`AnnotationLiteral`] for annotation arguments.
///
/// Note that `Ref` precedes `L` in the parameter list, because the default for
/// `L` is written in terms of `Ref` and a default cannot forward-reference a
/// later parameter.
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[cfg_attr(
	feature = "serde",
	serde(bound(serialize = "Identifier: Serialize, Lit: Serialize, Ref: FznRef"))
)]
pub enum Argument<Identifier = String, Ref: FznRef = Immutable, Lit = Literal<Identifier, Ref>> {
	/// Sequence of literals
	Array(Vec<Lit>),
	/// Named array of [`Literal`]s
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_array_ref::<_, _, Ref>",)
	)]
	ArrayNamed(Ref::Of<Array<Identifier, Ref>>),
	/// Literal
	Literal(Lit),
}

/// A definition of a named array literal in FlatZinc
///
/// FlatZinc Arrays are a simple (one-dimensional) sequence of [`Literal`]s.
/// These values are stored as the [`Array::contents`] member. Additional
/// information, in the form of [`Annotation`]s, from the MiniZinc model is
/// stored in [`Array::ann`] when present. When [`Array::defined`] is set to
/// `true`, then
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename = "array"))]
#[cfg_attr(
	feature = "serde",
	serde(bound(serialize = "Identifier: Serialize, Ref: FznRef"))
)]
#[derive(Clone, PartialEq, Debug)]
pub struct Array<Identifier = String, Ref: FznRef = Immutable> {
	/// The optional public name of the array literal.
	///
	/// This is `None` for arrays inlined within constraints.
	#[cfg_attr(feature = "serde", serde(skip))]
	pub name: String,
	/// The values stored within the array literal
	#[cfg_attr(feature = "serde", serde(rename = "a"))]
	pub contents: Vec<Literal<Identifier, Ref>>,
	/// List of annotations
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
	pub ann: Vec<Annotation<Identifier, Ref>>,
	/// This field is set to `true` when there is a constraint that has been
	/// marked as defining this array.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "serde_impl::is_false"))]
	pub defined: bool,
	/// This field is set to `true` when the array has been introduced by the
	/// MiniZinc compiler, rather than being explicitly defined at the top-level
	/// of the MiniZinc model.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "serde_impl::is_false"))]
	pub introduced: bool,
}

/// An object depicting a constraint
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename = "constraint"))]
#[cfg_attr(
	feature = "serde",
	serde(bound(serialize = "Identifier: Serialize, Ref: FznRef"))
)]
#[derive(Clone, PartialEq, Debug)]
pub struct Constraint<Identifier = String, Ref: FznRef = Immutable> {
	/// Identifier of the constraint predicate
	pub id: Identifier,
	/// Arguments of the constraint
	pub args: Vec<Argument<Identifier, Ref>>,
	/// Variable that the constraint defines
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub defines: Option<NamedRef<Identifier, Ref>>,
	/// List of annotations
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
	pub ann: Vec<Annotation<Identifier, Ref>>,
}

/// The structure depicting a FlatZinc instance
///
/// FlatZinc is (generally) a format produced by the MiniZinc compiler as a
/// result of instantiating the parameter variables of a MiniZinc model and
/// generating a solver-specific equisatisfiable model.
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(
	feature = "serde",
	serde(bound(serialize = "Identifier: Serialize, Ref: FznRef"))
)]
pub struct FlatZinc<Identifier = String, Ref: FznRef = Immutable> {
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_variable_map::<_, _, Ref>")
	)]
	/// A list of decision variable definitions.
	pub variables: Vec<Ref::Of<Variable<Identifier, Ref>>>,
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_array_map::<_, _, Ref>")
	)]
	/// A list of named array definitions.
	pub arrays: Vec<Ref::Of<Array<Identifier, Ref>>>,
	/// A list of (solver-specific) constraints, that must be satisfied in a
	/// solution.
	pub constraints: Vec<Constraint<Identifier, Ref>>,
	/// A list of all entities for which the solver must produce output for each
	/// solution.
	pub output: Vec<NamedRef<Identifier, Ref>>,
	/// A specification of the goal of solving the FlatZinc instance.
	pub solve: SolveObjective<Identifier, Ref>,
	/// The version of the FlatZinc serialization specification used
	pub version: String,
}

/// Literal values
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[cfg_attr(
	feature = "serde",
	serde(bound(serialize = "Identifier: Serialize, Ref: FznRef"))
)]
pub enum Literal<Identifier = String, Ref: FznRef = Immutable> {
	/// Integer value
	Int(i64),
	/// Floating point value
	Float(f64),
	/// Reference to a decision variable.
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_variable_ref::<_, _, Ref>",)
	)]
	Variable(Ref::Of<Variable<Identifier, Ref>>),
	/// Boolean value
	Bool(bool),
	/// Set of integers, represented as a list of integer ranges
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_set",)
	)]
	IntSet(RangeList<i64>),
	/// Set of floating point values, represented as a list of floating point
	/// ranges
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_set",)
	)]
	FloatSet(RangeList<f64>),
	/// String value
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_string",)
	)]
	String(String),
}

/// Goal of solving a FlatZinc instance.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Method<Identifier = String, Ref: FznRef = Immutable> {
	/// Find any solution.
	#[default]
	Satisfy,
	/// Find the solution with the lowest value for the given objective.
	Minimize(Literal<Identifier, Ref>),
	/// Find the solution with the highest value for the given objective.
	Maximize(Literal<Identifier, Ref>),
}

/// Reference to a named top-level declaration (variable or array)
///
/// ### Warning
///
/// It is possible for an [`Array`] to exist without an `name` attribute, if a
/// reference to such an [`Array`] is used as a [`NamedRef`], serialization can
/// panic.
///
/// [`NamedRef`] compares, hashes, and orders by the referenced declaration
/// name. As a consequence, two values that refer to different allocations but
/// expose the same name are considered equal, and a variable and array with
/// the same name are also treated as equal for these trait implementations.
/// Note that this cannot occur in valid FlatZinc models.
///
/// ### Warning
///
/// Under [`Mutable`], the [`Hash`], [`Ord`], and [`PartialEq`]
/// implementations take a read lock on the referenced declaration in order to
/// read its name. Invoking them while holding a write guard on that same
/// declaration will deadlock.
pub enum NamedRef<Identifier = String, Ref: FznRef = Immutable> {
	/// Reference to a variable.
	Variable(Ref::Of<Variable<Identifier, Ref>>),
	/// Reference to an array.
	Array(Ref::Of<Array<Identifier, Ref>>),
}

/// A specification of objective of a FlatZinc instance
#[derive(Clone, PartialEq, Debug)]
pub struct SolveObjective<Identifier = String, Ref: FznRef = Immutable> {
	/// The method expected to be used for solving the instance.
	pub method: Method<Identifier, Ref>,
	/// A list of annotations from the solve statement in the MiniZinc model
	///
	/// Note that this includes the search annotations if they are present in
	/// the model.
	pub ann: Vec<Annotation<Identifier, Ref>>,
}

/// Used to signal the type of (decision) [`Variable`]
#[derive(Clone, PartialEq, Debug)]
pub enum Type {
	/// Boolean decision variable
	Bool,
	/// Integer decision variable
	Int(Option<RangeList<i64>>),
	/// Floating point decision variable
	Float(Option<RangeList<f64>>),
	/// Integer set decision variable
	IntSet(Option<RangeList<i64>>),
}

/// The definition of a decision variable
#[derive(Clone, PartialEq, Debug)]
pub struct Variable<Identifier = String, Ref: FznRef = Immutable> {
	/// The public name of the decision variable.
	pub name: String,
	/// The type of the decision variable, and set of potential values  from
	/// which the decision variable must take its value in a solution, i.e. its
	/// domain.
	///
	/// If domain has the value `None`, then all values of the decision
	/// variable's `Type` are allowed in a solution.
	pub ty: Type,
	/// A list of annotations
	pub ann: Vec<Annotation<Identifier, Ref>>,
	/// This field is set to `true` when there is a constraint that has been
	/// marked as defining this variable.
	pub defined: bool,
	/// This field is set to `true` when the variable has been introduced by the
	/// MiniZinc compiler, rather than being explicitly defined at the top-level
	/// of the MiniZinc model.
	pub introduced: bool,
}

impl<Identifier: Display, Ref: FznRef> Display for Annotation<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Annotation::Atom(a) => write!(f, "{a}"),
			Annotation::Call(c) => write!(f, "{c}"),
		}
	}
}

impl<Identifier, Ref: FznRef> From<Argument<Identifier, Ref, Literal<Identifier, Ref>>>
	for Argument<Identifier, Ref, AnnotationLiteral<Identifier, Ref>>
{
	fn from(value: Argument<Identifier, Ref>) -> Self {
		match value {
			Argument::Array(arr) => Argument::Array(arr.into_iter().map(|l| l.into()).collect()),
			Argument::ArrayNamed(arr) => Argument::ArrayNamed(arr),
			Argument::Literal(l) => Argument::Literal(l.into()),
		}
	}
}

impl<Identifier: Clone, Ref: FznRef, L: Clone> Clone for Argument<Identifier, Ref, L> {
	fn clone(&self) -> Self {
		match self {
			Argument::Array(arr) => Argument::Array(arr.clone()),
			Argument::ArrayNamed(arr) => Argument::ArrayNamed(arr.clone()),
			Argument::Literal(lit) => Argument::Literal(lit.clone()),
		}
	}
}

impl<Identifier: Debug, Ref: FznRef, L: Debug> Debug for Argument<Identifier, Ref, L> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Argument::Array(arr) => f.debug_tuple("Array").field(arr).finish(),
			Argument::ArrayNamed(arr) => {
				Ref::with(arr, |arr| f.debug_tuple("ArrayNamed").field(arr).finish())
			}
			Argument::Literal(lit) => f.debug_tuple("Literal").field(lit).finish(),
		}
	}
}

impl<Identifier: PartialEq, Ref: FznRef, L: PartialEq> PartialEq for Argument<Identifier, Ref, L> {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Argument::Array(a), Argument::Array(b)) => a == b,
			(Argument::ArrayNamed(a), Argument::ArrayNamed(b)) => {
				Ref::addr(a) == Ref::addr(b) || Ref::with(a, |a| Ref::with(b, |b| a == b))
			}
			(Argument::Literal(a), Argument::Literal(b)) => a == b,
			_ => false,
		}
	}
}

impl<Identifier: Display, Ref: FznRef> Display for AnnotationCall<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}(", self.id)?;
		let mut first = true;
		for arg in &self.args {
			if !first {
				write!(f, ", ")?
			}
			write!(f, "{arg}")?;
			first = false;
		}
		write!(f, ")")
	}
}

impl<Identifier: Display, Ref: FznRef> Display for AnnotationLiteral<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			AnnotationLiteral::Literal(lit) => write!(f, "{lit}"),
			AnnotationLiteral::Annotation(ann) => write!(f, "{ann}"),
		}
	}
}

impl<Identifier, Ref: FznRef> From<Literal<Identifier, Ref>>
	for AnnotationLiteral<Identifier, Ref>
{
	fn from(value: Literal<Identifier, Ref>) -> Self {
		AnnotationLiteral::Literal(value)
	}
}

impl<Identifier: Display, Ref: FznRef, L: Display> Display for Argument<Identifier, Ref, L> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Argument::Array(arr) => {
				write!(f, "[")?;
				let mut first = true;
				for v in arr {
					if !first {
						write!(f, ", ")?
					}
					write!(f, "{v}")?;
					first = false;
				}
				write!(f, "]")
			}
			Argument::ArrayNamed(arr) => Ref::with(arr, |arr| write!(f, "{}", arr.name)),
			Argument::Literal(lit) => write!(f, "{lit}"),
		}
	}
}

impl<Identifier, Ref: FznRef> Array<Identifier, Ref> {
	/// Clones this array reference into an [`ArcKey`].
	///
	/// This is useful when storing arrays in collections such as
	/// [`HashMap`](std::collections::HashMap), [`HashSet`], and
	/// [`BTreeMap`](std::collections::BTreeMap), where the key should identify
	/// the specific parsed array object rather than its contents or `name`.
	///
	/// This method clones the [`Arc`] reference count and keeps the original
	/// array reference usable by the caller.
	///
	/// The resulting key uses the allocation / pointer identity of this
	/// [`Arc`]. Two arrays with the same name and equal contents will
	/// therefore compare as different keys if they are stored in different
	/// allocations.
	///
	/// During FlatZinc parsing and deserialization, this crate guarantees that
	/// identical top-level arrays are allocated only once. In those cases,
	/// `ArcKey` is a good fit for keying collections by the canonical parsed
	/// array object.
	pub fn cloned_key(self: &Arc<Self>) -> ArcKey<Self> {
		ArcKey::new(Arc::clone(self))
	}

	/// Heuristic to determine the type of the array
	fn determine_type(&self) -> (&str, bool) {
		let ty = match self.contents.first().unwrap() {
			Literal::Int(_) => "int",
			Literal::Float(_) => "float",
			Literal::Variable(var) => return (Ref::with(var, |var| var.ty.base_name()), true),
			Literal::Bool(_) => "bool",
			Literal::IntSet(_) => "set of int",
			Literal::FloatSet(_) => "set of float",
			Literal::String(_) => "string",
		};
		let is_var = self
			.contents
			.iter()
			.any(|lit| matches!(lit, Literal::Variable(_)));
		(ty, is_var)
	}
}

impl<Identifier: Display, Ref: FznRef> Display for Constraint<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}(", self.id)?;
		let mut first = true;
		for arg in &self.args {
			if !first {
				write!(f, ", ")?
			}
			write!(f, "{arg}")?;
			first = false;
		}
		write!(f, ")")?;
		if let Some(defines) = &self.defines {
			write!(f, " ::defines_var({})", defines.name())?
		}
		for a in &self.ann {
			write!(f, " ::{a}")?
		}
		Ok(())
	}
}

impl<Identifier, Ref: FznRef> FlatZinc<Identifier, Ref>
where
	Identifier: Clone + Debug,
{
	/// Deserialize a FlatZinc JSON value using a custom identifier interner,
	/// used for constraint and annotation identifiers.
	///
	/// Unknown fields on inner FlatZinc objects are rejected. Unknown fields on
	/// the outer top-level JSON object are ignored.
	#[cfg(feature = "serde")]
	pub fn deserialize_with_interner<'de, D, F, E>(
		deserializer: D,
		interner: F,
	) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
		F: FnMut(&str) -> Result<Identifier, E>,
		E: Display,
	{
		use serde::de::{self, DeserializeSeed};

		use crate::intermediate::ParserState;

		let (model, interner) = ParserState::new(interner).deserialize(deserializer)?;
		FlatZinc::from_intermediate(model, interner).map_err(de::Error::custom)
	}

	/// Parse a `.fzn` source into a [`FlatZinc`] instance.
	#[cfg(feature = "fzn")]
	pub fn from_fzn<E>(source: impl std::io::BufRead) -> Result<Self, FznParseError>
	where
		for<'a> Identifier: TryFrom<&'a str, Error = E>,
		E: Display,
	{
		fzn::parse(source)
	}

	/// Parse a `.fzn` source into a [`FlatZinc`] instance using a custom
	/// identifier interner, used for constraint and annotation identifiers.
	#[cfg(feature = "fzn")]
	pub fn from_fzn_with_interner<F, E>(
		source: impl std::io::BufRead,
		interner: F,
	) -> Result<Self, FznParseError>
	where
		F: FnMut(&str) -> Result<Identifier, E>,
		E: Display,
	{
		fzn::parse_with_interner(source, interner)
	}
}

impl<Identifier: Clone, Ref: FznRef> Clone for FlatZinc<Identifier, Ref> {
	fn clone(&self) -> Self {
		Self {
			variables: self.variables.clone(),
			arrays: self.arrays.clone(),
			constraints: self.constraints.clone(),
			output: self.output.clone(),
			solve: self.solve.clone(),
			version: self.version.clone(),
		}
	}
}

impl<Identifier: Debug, Ref: FznRef> Debug for FlatZinc<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		/// Wrapper printing a slice of shared declarations as if the references
		/// were transparent, matching the derived output under
		/// [`Immutable`](helpers::Immutable).
		struct Declarations<'a, T, Ref: FznRef>(&'a [Ref::Of<T>]);

		impl<T: Debug, Ref: FznRef> Debug for Declarations<'_, T, Ref> {
			fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				let mut list = f.debug_list();
				for node in self.0 {
					let _ = Ref::with(node, |node| list.entry(node));
				}
				list.finish()
			}
		}

		f.debug_struct("FlatZinc")
			.field(
				"variables",
				&Declarations::<_, Ref>(self.variables.as_slice()),
			)
			.field("arrays", &Declarations::<_, Ref>(self.arrays.as_slice()))
			.field("constraints", &self.constraints)
			.field("output", &self.output)
			.field("solve", &self.solve)
			.field("version", &self.version)
			.finish()
	}
}

impl<Identifier, Ref: FznRef> Default for FlatZinc<Identifier, Ref> {
	fn default() -> Self {
		Self {
			variables: Vec::new(),
			arrays: Vec::new(),
			constraints: Vec::new(),
			output: Vec::new(),
			solve: Default::default(),
			version: "1.0".into(),
		}
	}
}

impl<Identifier: PartialEq, Ref: FznRef> PartialEq for FlatZinc<Identifier, Ref> {
	fn eq(&self, other: &Self) -> bool {
		/// Compare two slices of shared declarations by content,
		/// short-circuiting on pointer identity to avoid locking the same
		/// declaration twice.
		fn eq_declarations<T: PartialEq, Ref: FznRef>(a: &[Ref::Of<T>], b: &[Ref::Of<T>]) -> bool {
			a.len() == b.len()
				&& a.iter().zip(b).all(|(a, b)| {
					Ref::addr(a) == Ref::addr(b) || Ref::with(a, |a| Ref::with(b, |b| a == b))
				})
		}

		eq_declarations::<_, Ref>(&self.variables, &other.variables)
			&& eq_declarations::<_, Ref>(&self.arrays, &other.arrays)
			&& self.constraints == other.constraints
			&& self.output == other.output
			&& self.solve == other.solve
			&& self.version == other.version
	}
}

impl<Identifier: Display, Ref: FznRef> Display for FlatZinc<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let output_map: HashSet<_> = self.output.iter().collect();

		for node in &self.variables {
			let name_ref = NamedRef::Variable(node.clone());
			let in_output = output_map.contains(&name_ref);
			Ref::with(node, |var| {
				write!(f, "var {}", var.ty)?;
				write!(f, ": {}", var.name)?;
				if in_output {
					write!(f, " ::output_var")?;
				}
				if var.defined {
					write!(f, " ::is_defined_var")?;
				}
				if var.introduced {
					write!(f, " ::var_is_introduced")?;
				}
				for ann in &var.ann {
					write!(f, " ::{ann}")?
				}
				writeln!(f, ";")
			})?
		}
		for node in &self.arrays {
			let name_ref = NamedRef::Array(node.clone());
			let in_output = output_map.contains(&name_ref);
			Ref::with(node, |arr| {
				let (ty, is_var) = arr.determine_type();
				write!(
					f,
					"array[1..{}] of {}{ty}: {}",
					arr.contents.len(),
					if is_var { "var " } else { "" },
					arr.name
				)?;
				if in_output {
					write!(f, " ::output_array([1..{}])", arr.contents.len())?;
				}
				if arr.defined {
					write!(f, " ::is_defined_var")?;
				}
				if arr.introduced {
					write!(f, " ::var_is_introduced")?;
				}
				for ann in &arr.ann {
					write!(f, " ::{ann}")?
				}
				write!(f, " = [")?;
				let mut first = true;
				for v in &arr.contents {
					if !first {
						write!(f, ", ")?;
					}
					write!(f, "{v}")?;
					first = false;
				}
				writeln!(f, "];")
			})?
		}
		for c in &self.constraints {
			writeln!(f, "constraint {c};")?;
		}
		writeln!(f, "{};", self.solve)
	}
}

impl<Identifier: Clone, Ref: FznRef> Clone for Literal<Identifier, Ref> {
	fn clone(&self) -> Self {
		match self {
			Literal::Int(i) => Literal::Int(*i),
			Literal::Float(x) => Literal::Float(*x),
			Literal::Variable(var) => Literal::Variable(var.clone()),
			Literal::Bool(b) => Literal::Bool(*b),
			Literal::IntSet(is) => Literal::IntSet(is.clone()),
			Literal::FloatSet(fs) => Literal::FloatSet(fs.clone()),
			Literal::String(s) => Literal::String(s.clone()),
		}
	}
}

impl<Identifier: Debug, Ref: FznRef> Debug for Literal<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Literal::Int(i) => f.debug_tuple("Int").field(i).finish(),
			Literal::Float(x) => f.debug_tuple("Float").field(x).finish(),
			Literal::Variable(var) => {
				Ref::with(var, |var| f.debug_tuple("Variable").field(var).finish())
			}
			Literal::Bool(b) => f.debug_tuple("Bool").field(b).finish(),
			Literal::IntSet(is) => f.debug_tuple("IntSet").field(is).finish(),
			Literal::FloatSet(fs) => f.debug_tuple("FloatSet").field(fs).finish(),
			Literal::String(s) => f.debug_tuple("String").field(s).finish(),
		}
	}
}

impl<Identifier: Display, Ref: FznRef> Display for Literal<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Literal::Int(i) => write!(f, "{i}"),
			Literal::Float(x) => write!(f, "{x:?}"),
			Literal::Variable(var) => Ref::with(var, |var| write!(f, "{}", var.name)),
			Literal::Bool(b) => write!(f, "{b}"),
			Literal::IntSet(is) => write!(f, "{is}"),
			Literal::FloatSet(fs) => write!(f, "{fs}"),
			Literal::String(s) => write!(f, "{s:?}"),
		}
	}
}

impl<Identifier: PartialEq, Ref: FznRef> PartialEq for Literal<Identifier, Ref> {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Literal::Int(a), Literal::Int(b)) => a == b,
			(Literal::Float(a), Literal::Float(b)) => a == b,
			(Literal::Variable(a), Literal::Variable(b)) => {
				Ref::addr(a) == Ref::addr(b) || Ref::with(a, |a| Ref::with(b, |b| a == b))
			}
			(Literal::Bool(a), Literal::Bool(b)) => a == b,
			(Literal::IntSet(a), Literal::IntSet(b)) => a == b,
			(Literal::FloatSet(a), Literal::FloatSet(b)) => a == b,
			(Literal::String(a), Literal::String(b)) => a == b,
			_ => false,
		}
	}
}

impl<Identifier: Display, Ref: FznRef> Display for Method<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Method::Satisfy => write!(f, "satisfy"),
			Method::Minimize(objective) => write!(f, "minimize {objective}"),
			Method::Maximize(objective) => write!(f, "maximize {objective}"),
		}
	}
}

impl<Identifier, Ref: FznRef> NamedRef<Identifier, Ref> {
	/// Return the identifier of the referenced output target.
	///
	/// The name is borrowed under [`Immutable`], but must be
	/// cloned under [`Mutable`], where it lives behind a lock.
	pub fn name(&self) -> Cow<'_, str> {
		match self {
			NamedRef::Variable(var) => Ref::map_str(var, |var| &var.name),
			NamedRef::Array(array) => Ref::map_str(array, |array| &array.name),
		}
	}
}

impl<Identifier, Ref: FznRef> Clone for NamedRef<Identifier, Ref> {
	fn clone(&self) -> Self {
		match self {
			NamedRef::Variable(var) => NamedRef::Variable(var.clone()),
			NamedRef::Array(array) => NamedRef::Array(array.clone()),
		}
	}
}

impl<Identifier: Debug, Ref: FznRef> Debug for NamedRef<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			NamedRef::Variable(var) => {
				Ref::with(var, |var| f.debug_tuple("Variable").field(var).finish())
			}
			NamedRef::Array(array) => {
				Ref::with(array, |array| f.debug_tuple("Array").field(array).finish())
			}
		}
	}
}

impl<Identifier, Ref: FznRef> Eq for NamedRef<Identifier, Ref> {}

// These conversions cannot be written generically over `Ref`: an associated
// type is opaque, so the compiler cannot rule out that `Ref::Of<Array<..>>` and
// `Ref::Of<Variable<..>>` name the same type for some implementor, and the two
// impls would overlap. They are therefore spelled out per marker. Generic code
// constructs the variants directly instead.

impl<Identifier> From<Arc<Array<Identifier, Immutable>>> for NamedRef<Identifier, Immutable> {
	fn from(node: Arc<Array<Identifier, Immutable>>) -> Self {
		NamedRef::Array(node)
	}
}

impl<Identifier> From<Arc<Variable<Identifier, Immutable>>> for NamedRef<Identifier, Immutable> {
	fn from(node: Arc<Variable<Identifier, Immutable>>) -> Self {
		NamedRef::Variable(node)
	}
}

impl<Identifier> From<Arc<RwLock<Array<Identifier, Mutable>>>> for NamedRef<Identifier, Mutable> {
	fn from(node: Arc<RwLock<Array<Identifier, Mutable>>>) -> Self {
		NamedRef::Array(node)
	}
}

impl<Identifier> From<Arc<RwLock<Variable<Identifier, Mutable>>>>
	for NamedRef<Identifier, Mutable>
{
	fn from(node: Arc<RwLock<Variable<Identifier, Mutable>>>) -> Self {
		NamedRef::Variable(node)
	}
}

impl<Identifier, Ref: FznRef> Hash for NamedRef<Identifier, Ref> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.name().hash(state);
	}
}

impl<Identifier, Ref: FznRef> Ord for NamedRef<Identifier, Ref> {
	fn cmp(&self, other: &Self) -> Ordering {
		self.name().cmp(&other.name())
	}
}

impl<Identifier, Ref: FznRef> PartialEq for NamedRef<Identifier, Ref> {
	fn eq(&self, other: &Self) -> bool {
		self.name() == other.name()
	}
}

impl<Identifier, Ref: FznRef> PartialOrd for NamedRef<Identifier, Ref> {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl<Identifier, Ref: FznRef> Default for SolveObjective<Identifier, Ref> {
	fn default() -> Self {
		Self {
			method: Default::default(),
			ann: Vec::new(),
		}
	}
}

impl<Identifier: Display, Ref: FznRef> Display for SolveObjective<Identifier, Ref> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "solve ")?;
		for a in &self.ann {
			write!(f, "::{a} ")?;
		}
		write!(f, "{}", self.method)
	}
}

impl Type {
	/// Return the canonical FlatZinc type name without any domain restriction.
	fn base_name(&self) -> &'static str {
		match self {
			Type::Bool => "bool",
			Type::Int(_) => "int",
			Type::Float(_) => "float",
			Type::IntSet(_) => "set of int",
		}
	}
}

impl Display for Type {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Type::Bool => write!(f, "bool"),
			Type::Int(Some(domain)) => write!(f, "{domain}"),
			Type::Int(None) => write!(f, "int"),
			Type::Float(Some(domain)) => write!(f, "{domain}"),
			Type::Float(None) => write!(f, "float"),
			Type::IntSet(Some(domain)) => write!(f, "set of {domain}"),
			Type::IntSet(None) => write!(f, "set of int"),
		}
	}
}

impl<Identifier, Ref: FznRef> Variable<Identifier, Ref> {
	/// Clones this variable reference into an [`ArcKey`].
	///
	/// This is useful when storing variables in collections such as
	/// [`HashMap`](std::collections::HashMap), [`HashSet`], and
	/// [`BTreeMap`](std::collections::BTreeMap), where the key should identify
	/// the specific parsed variable object rather than its fields or `name`.
	///
	/// This method clones the [`Arc`] reference count and keeps the original
	/// variable reference usable by the caller.
	///
	/// The resulting key uses the allocation / pointer identity of this
	/// [`Arc`]. Two variables with the same name and equal fields will
	/// therefore compare as different keys if they are stored in different
	/// allocations.
	///
	/// During FlatZinc parsing and deserialization, this crate guarantees that
	/// identical top-level variables are allocated only once. In those cases,
	/// `ArcKey` is a good fit for keying collections by the canonical parsed
	/// variable object.
	pub fn cloned_key(self: &Arc<Self>) -> ArcKey<Self> {
		ArcKey::new(Arc::clone(self))
	}
}
