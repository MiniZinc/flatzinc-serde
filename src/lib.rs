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
	cmp::Ordering,
	collections::HashSet,
	fmt::{Debug, Display},
	hash::{Hash, Hasher},
	sync::{Arc, Weak},
};

pub use rangelist::RangeList;
#[cfg(feature = "serde")]
use serde::{Deserializer, Serialize};

pub use crate::error::{FznParseError, LinkError};
use crate::helpers::ArcKey;

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
#[derive(Clone, PartialEq, Debug)]
pub enum Annotation<Identifier = String> {
	/// Atom annotation (i.e., a single `Identifier`)
	Atom(Identifier),
	/// Call annotation
	Call(AnnotationCall<Identifier>),
}

/// The argument type associated with [`AnnotationCall`]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[derive(Clone, Debug)]
pub enum AnnotationArgument<Identifier = String> {
	/// Sequence of [`Literal`]s
	Array(Vec<AnnotationLiteral<Identifier>>),
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_array_weak")
	)]
	/// Named array of [`Literal`]s
	ArrayNamed(Weak<Array<Identifier>>),
	/// Singular argument
	Literal(AnnotationLiteral<Identifier>),
}

/// An object depicting an annotation in the form of a call
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename = "annotation_call"))]
#[derive(Clone, PartialEq, Debug)]
pub struct AnnotationCall<Identifier = String> {
	/// Identifier of the constraint predicate
	pub id: Identifier,
	/// Arguments of the constraint
	pub args: Vec<AnnotationArgument<Identifier>>,
}

#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[derive(Clone, Debug)]
/// Literal values as arguments to [`AnnotationCall`]
pub enum AnnotationLiteral<Identifier = String> {
	/// Integer value
	Int(i64),
	/// Floating point value
	Float(f64),
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_variable_weak")
	)]
	/// Reference to a decision variable.
	Variable(Weak<Variable<Identifier>>),
	/// Boolean value
	Bool(bool),
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_set")
	)]
	/// Set of integers, represented as a list of integer ranges
	IntSet(RangeList<i64>),
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_set")
	)]
	/// Set of floating point values, represented as a list of floating point
	/// ranges
	FloatSet(RangeList<f64>),
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_string")
	)]
	/// String value
	String(String),
	/// An annotation object.
	Annotation(Annotation<Identifier>),
}

/// The argument type associated with [`Constraint`]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[derive(Clone, PartialEq, Debug)]
pub enum Argument<Identifier = String> {
	/// Sequence of [`Literal`]s
	Array(Vec<Literal<Identifier>>),
	/// Sequence of [`Literal`]s
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_array_arc",)
	)]
	ArrayNamed(Arc<Array<Identifier>>),
	/// Literal
	Literal(Literal<Identifier>),
}

#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename = "array"))]
#[derive(Clone, PartialEq, Debug)]
/// A definition of a named array literal in FlatZinc
///
/// FlatZinc Arrays are a simple (one-dimensional) sequence of [`Literal`]s.
/// These values are stored as the [`Array::contents`] member. Additional
/// information, in the form of [`Annotation`]s, from the MiniZinc model is
/// stored in [`Array::ann`] when present. When [`Array::defined`] is set to
/// `true`, then
pub struct Array<Identifier = String> {
	#[cfg_attr(feature = "serde", serde(skip))]
	/// The optional public name of the array literal.
	///
	/// This is `None` for arrays inlined within constraints.
	pub name: String,
	#[cfg_attr(feature = "serde", serde(rename = "a"))]
	/// The values stored within the array literal
	pub contents: Vec<Literal<Identifier>>,
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
	/// List of annotations
	pub ann: Vec<Annotation<Identifier>>,
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "serde_impl::is_false"))]
	/// This field is set to `true` when there is a constraint that has been
	/// marked as defining this array.
	pub defined: bool,
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "serde_impl::is_false"))]
	/// This field is set to `true` when the array has been introduced by the
	/// MiniZinc compiler, rather than being explicitly defined at the top-level
	/// of the MiniZinc model.
	pub introduced: bool,
}

#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename = "constraint"))]
#[derive(Clone, PartialEq, Debug)]
/// An object depicting a constraint
pub struct Constraint<Identifier = String> {
	/// Identifier of the constraint predicate
	pub id: Identifier,
	/// Arguments of the constraint
	pub args: Vec<Argument<Identifier>>,
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	/// Variable that the constraint defines
	pub defines: Option<NamedRef<Identifier>>,
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
	/// List of annotations
	pub ann: Vec<Annotation<Identifier>>,
}

#[cfg_attr(feature = "serde", derive(Serialize))]
#[derive(Clone, PartialEq, Debug)]
/// The structure depicting a FlatZinc instance
///
/// FlatZinc is (generally) a format produced by the MiniZinc compiler as a
/// result of instantiating the parameter variables of a MiniZinc model and
/// generating a solver-specific equisatisfiable model.
///
/// During parsing, any variable right-hand side declarations are resolved
/// eagerly. The resulting public model stores only non-aliased variables, while
/// references in constraints, arrays, and objectives are rewritten to the
/// resolved literals.
pub struct FlatZinc<Identifier = String> {
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_variable_map")
	)]
	/// A list of decision variable definitions.
	pub variables: Vec<Arc<Variable<Identifier>>>,
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_array_map")
	)]
	/// A list of named array definitions.
	pub arrays: Vec<Arc<Array<Identifier>>>,
	/// A list of (solver-specific) constraints, that must be satisfied in a
	/// solution.
	pub constraints: Vec<Constraint<Identifier>>,
	/// A list of all entities for which the solver must produce output for each
	/// solution.
	pub output: Vec<NamedRef<Identifier>>,
	/// A specification of the goal of solving the FlatZinc instance.
	pub solve: SolveObjective<Identifier>,
	/// The version of the FlatZinc serialization specification used
	pub version: String,
}

#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[derive(Clone, PartialEq, Debug)]
/// Literal values
pub enum Literal<Identifier = String> {
	/// Integer value
	Int(i64),
	/// Floating point value
	Float(f64),
	/// Reference to a decision variable.
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_variable_arc",)
	)]
	Variable(Arc<Variable<Identifier>>),
	/// Boolean value
	Bool(bool),
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_set",)
	)]
	/// Set of integers, represented as a list of integer ranges
	IntSet(RangeList<i64>),
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_set",)
	)]
	/// Set of floating point values, represented as a list of floating point
	/// ranges
	FloatSet(RangeList<f64>),
	#[cfg_attr(
		feature = "serde",
		serde(serialize_with = "serde_impl::serialize_encapsulate_string",)
	)]
	/// String value
	String(String),
}

/// Goal of solving a FlatZinc instance.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Method<Identifier = String> {
	#[default]
	/// Find any solution.
	Satisfy,
	/// Find the solution with the lowest value for the given objective.
	Minimize(Literal<Identifier>),
	/// Find the solution with the highest value for the given objective.
	Maximize(Literal<Identifier>),
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
#[derive(Clone, Debug)]
pub enum NamedRef<Identifier = String> {
	/// Reference to a variable.
	Variable(Arc<Variable<Identifier>>),
	/// Reference to an array.
	Array(Arc<Array<Identifier>>),
}

/// A specification of objective of a FlatZinc instance
#[derive(Clone, PartialEq, Debug)]
pub struct SolveObjective<Identifier = String> {
	/// The method expected to be used for solving the instance.
	pub method: Method<Identifier>,
	/// A list of annotations from the solve statement in the MiniZinc model
	///
	/// Note that this includes the search annotations if they are present in
	/// the model.
	pub ann: Vec<Annotation<Identifier>>,
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
///
/// Any right-hand side declarations from the FlatZinc input are resolved during
/// parsing and are therefore not stored on the public type. Standalone JSON
/// deserialization of [`Variable`] values does not accept an `rhs` field.
#[derive(Clone, PartialEq, Debug)]
pub struct Variable<Identifier = String> {
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
	pub ann: Vec<Annotation<Identifier>>,
	/// This field is set to `true` when there is a constraint that has been
	/// marked as defining this variable.
	pub defined: bool,
	/// This field is set to `true` when the variable has been introduced by the
	/// MiniZinc compiler, rather than being explicitly defined at the top-level
	/// of the MiniZinc model.
	pub introduced: bool,
}

impl<Identifier: Display> Display for Annotation<Identifier> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Annotation::Atom(a) => write!(f, "{a}"),
			Annotation::Call(c) => write!(f, "{c}"),
		}
	}
}

impl<Idenfier: Display> Display for AnnotationArgument<Idenfier> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			AnnotationArgument::Array(arr) => {
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
			AnnotationArgument::ArrayNamed(array) => match array.upgrade() {
				Some(array) => write!(f, "{}", &array.name),
				None => write!(f, "[/* dangling array ref */]"),
			},
			AnnotationArgument::Literal(lit) => write!(f, "{lit}"),
		}
	}
}

impl<Identifier> From<Argument<Identifier>> for AnnotationArgument<Identifier> {
	fn from(value: Argument<Identifier>) -> Self {
		match value {
			Argument::Array(arr) => {
				AnnotationArgument::Array(arr.into_iter().map(|l| l.into()).collect())
			}
			Argument::ArrayNamed(arr) => AnnotationArgument::ArrayNamed(Arc::downgrade(&arr)),
			Argument::Literal(l) => AnnotationArgument::Literal(l.into()),
		}
	}
}

impl<Identifier: PartialEq> PartialEq for AnnotationArgument<Identifier> {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(AnnotationArgument::Array(lhs), AnnotationArgument::Array(rhs)) => lhs == rhs,
			(AnnotationArgument::ArrayNamed(lhs), AnnotationArgument::ArrayNamed(rhs)) => {
				match (lhs.upgrade(), rhs.upgrade()) {
					(Some(lhs), Some(rhs)) => lhs == rhs,
					(None, None) => true,
					_ => false,
				}
			}
			(AnnotationArgument::Literal(lhs), AnnotationArgument::Literal(rhs)) => lhs == rhs,
			_ => false,
		}
	}
}

impl<Identifier: Display> Display for AnnotationCall<Identifier> {
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

impl<Idenfier: Display> Display for AnnotationLiteral<Idenfier> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			AnnotationLiteral::Int(i) => write!(f, "{i}"),
			AnnotationLiteral::Float(x) => write!(f, "{x:?}"),
			AnnotationLiteral::Variable(var) => match var.upgrade() {
				Some(var) => write!(f, "{}", var.name),
				None => write!(f, "DANGLING_VARIABLE_REFERENCE"),
			},
			AnnotationLiteral::Bool(b) => write!(f, "{b}"),
			AnnotationLiteral::IntSet(is) => write!(f, "{is}"),
			AnnotationLiteral::FloatSet(fs) => write!(f, "{fs}"),
			AnnotationLiteral::String(s) => write!(f, "{s:?}"),
			AnnotationLiteral::Annotation(ann) => write!(f, "{ann}"),
		}
	}
}

impl<Identifier> From<Literal<Identifier>> for AnnotationLiteral<Identifier> {
	fn from(value: Literal<Identifier>) -> Self {
		match value {
			Literal::Int(i) => AnnotationLiteral::Int(i),
			Literal::Float(f) => AnnotationLiteral::Float(f),
			Literal::Bool(b) => AnnotationLiteral::Bool(b),
			Literal::String(s) => AnnotationLiteral::String(s),
			Literal::Variable(var) => AnnotationLiteral::Variable(Arc::downgrade(&var)),
			Literal::IntSet(set) => AnnotationLiteral::IntSet(set),
			Literal::FloatSet(set) => AnnotationLiteral::FloatSet(set),
		}
	}
}

impl<Identifier: PartialEq> PartialEq for AnnotationLiteral<Identifier> {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Self::Int(lhs), Self::Int(rhs)) => lhs == rhs,
			(Self::Float(lhs), Self::Float(rhs)) => lhs == rhs,
			(Self::Variable(lhs), Self::Variable(rhs)) => match (lhs.upgrade(), rhs.upgrade()) {
				(Some(lhs), Some(rhs)) => lhs == rhs,
				(None, None) => true,
				_ => false,
			},
			(Self::Bool(lhs), Self::Bool(rhs)) => lhs == rhs,
			(Self::IntSet(lhs), Self::IntSet(rhs)) => lhs == rhs,
			(Self::FloatSet(lhs), Self::FloatSet(rhs)) => lhs == rhs,
			(Self::String(lhs), Self::String(rhs)) => lhs == rhs,
			(Self::Annotation(lhs), Self::Annotation(rhs)) => lhs == rhs,
			_ => false,
		}
	}
}
impl<Identifier: Display> Display for Argument<Identifier> {
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
			Argument::ArrayNamed(arr) => write!(f, "{}", &arr.name),
			Argument::Literal(lit) => write!(f, "{lit}"),
		}
	}
}

impl<Identifier> Array<Identifier> {
	/// Heuristic to determine the type of the array
	fn determine_type(&self) -> (&str, bool) {
		let ty = match self.contents.first().unwrap() {
			Literal::Int(_) => "int",
			Literal::Float(_) => "float",
			Literal::Variable(var) => return (var.ty.base_name(), true),
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

	/// Converts an array reference into an [`ArcKey`](crate::helpers::ArcKey).
	///
	/// This is useful when storing arrays in collections such as
	/// [`HashMap`](std::collections::HashMap),
	/// [`HashSet`](std::collections::HashSet), and
	/// [`BTreeMap`](std::collections::BTreeMap), where the key should identify
	/// the specific parsed array object rather than its contents or `name`.
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
	pub fn into_key(self: Arc<Self>) -> ArcKey<Self> {
		ArcKey::new(self)
	}
}

impl<Identifier: Display> Display for Constraint<Identifier> {
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

impl<Identifier> FlatZinc<Identifier>
where
	Identifier: Clone + Debug,
{
	#[cfg(feature = "serde")]
	/// Deserialize a FlatZinc JSON value using a custom identifier interner.
	///
	/// Variable right-hand side declarations are resolved eagerly, so aliased
	/// variables are omitted from the returned [`FlatZinc::variables`] map.
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

	#[cfg(feature = "fzn")]
	/// Parse a `.fzn` source into a [`FlatZinc`] instance.
	pub fn from_fzn<E>(source: impl std::io::BufRead) -> Result<Self, FznParseError>
	where
		for<'a> Identifier: TryFrom<&'a str, Error = E>,
		E: Display,
	{
		fzn::parse(source)
	}

	#[cfg(feature = "fzn")]
	/// Parse a `.fzn` source into a [`FlatZinc`] instance using a custom
	/// identifier interner.
	///
	/// Variable right-hand side declarations are resolved eagerly, so aliased
	/// variables are omitted from the returned [`FlatZinc::variables`] map.
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

impl<Identifier> Default for FlatZinc<Identifier> {
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

impl<Identifier: Display> Display for FlatZinc<Identifier> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let output_map: HashSet<_> = self.output.iter().collect();

		for var in &self.variables {
			write!(f, "var {}", var.ty)?;
			write!(f, ": {}", var.name)?;
			let name_ref: NamedRef<_> = Arc::clone(var).into();
			if output_map.contains(&name_ref) {
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
			writeln!(f, ";")?
		}
		for arr in &self.arrays {
			let (ty, is_var) = arr.determine_type();
			write!(
				f,
				"array[1..{}] of {}{ty}: {}",
				arr.contents.len(),
				if is_var { "var " } else { "" },
				arr.name
			)?;
			let name_ref: NamedRef<_> = Arc::clone(arr).into();
			if output_map.contains(&name_ref) {
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
			writeln!(f, "];")?
		}
		for c in &self.constraints {
			writeln!(f, "constraint {c};")?;
		}
		writeln!(f, "{};", self.solve)
	}
}

impl<Identifier: Display> Display for Literal<Identifier> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Literal::Int(i) => write!(f, "{i}"),
			Literal::Float(x) => write!(f, "{x:?}"),
			Literal::Variable(var) => write!(f, "{}", var.name),
			Literal::Bool(b) => write!(f, "{b}"),
			Literal::IntSet(is) => write!(f, "{is}"),
			Literal::FloatSet(fs) => write!(f, "{fs}"),
			Literal::String(s) => write!(f, "{s:?}"),
		}
	}
}

impl<Identifier: Display> Display for Method<Identifier> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Method::Satisfy => write!(f, "satisfy"),
			Method::Minimize(objective) => write!(f, "minimize {objective}"),
			Method::Maximize(objective) => write!(f, "maximize {objective}"),
		}
	}
}

impl<Identifier> NamedRef<Identifier> {
	/// Return the identifier of the referenced output target.
	pub fn name(&self) -> &str {
		match self {
			NamedRef::Variable(var) => &var.name,
			NamedRef::Array(array) => &array.name,
		}
	}
}

impl<Identifier> Eq for NamedRef<Identifier> {}

impl<Identifier> From<Arc<Array<Identifier>>> for NamedRef<Identifier> {
	fn from(arc: Arc<Array<Identifier>>) -> Self {
		NamedRef::Array(arc)
	}
}

impl<Identifier> From<Arc<Variable<Identifier>>> for NamedRef<Identifier> {
	fn from(arc: Arc<Variable<Identifier>>) -> Self {
		NamedRef::Variable(arc)
	}
}

impl<Identifier> Hash for NamedRef<Identifier> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.name().hash(state);
	}
}

impl<Identifier> Ord for NamedRef<Identifier> {
	fn cmp(&self, other: &Self) -> Ordering {
		self.name().cmp(other.name())
	}
}

impl<Identifier> PartialEq for NamedRef<Identifier> {
	fn eq(&self, other: &Self) -> bool {
		self.name() == other.name()
	}
}

impl<Identifier> PartialOrd for NamedRef<Identifier> {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl<Identifier> Default for SolveObjective<Identifier> {
	fn default() -> Self {
		Self {
			method: Default::default(),
			ann: Vec::new(),
		}
	}
}

impl<Identifier: Display> Display for SolveObjective<Identifier> {
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

impl<Identifier> Variable<Identifier> {
	/// Converts a variable reference into an
	/// [`ArcKey`](crate::helpers::ArcKey).
	///
	/// This is useful when storing variables in collections such as
	/// [`HashMap`](std::collections::HashMap),
	/// [`HashSet`](std::collections::HashSet), and
	/// [`BTreeMap`](std::collections::BTreeMap), where the key should identify
	/// the specific parsed variable object rather than its fields or `name`.
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
	pub fn into_key(self: Arc<Self>) -> ArcKey<Self> {
		ArcKey::new(self)
	}
}
