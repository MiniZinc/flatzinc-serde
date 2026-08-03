//! Transient parse-time representations used before linking model references.
//!
//! The intermediate model stores each model-level name exactly once inside a
//! single slot table. Each slot owns the canonical name text and tracks whether
//! that name has been declared as a variable, declared as an array, or is still
//! unresolved.

use std::{
	collections::HashMap,
	fmt::{Debug, Display, Formatter},
	mem,
};

use crate::{
	LinkError, NamedRef, RangeList, Type,
	helpers::{FznRef, Immutable},
};

/// An intermediate annotation.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Annotation<Identifier> {
	/// Atom annotation.
	Atom(Identifier),
	/// Call annotation.
	Call(AnnotationCall<Identifier>),
}

/// An intermediate annotation argument.
///
/// A crate-internal alias for the [`Argument`] instantiation used by
/// annotations, whose literals are [`AnnotationLiteral`]s (i.e., they may
/// additionally be nested annotations). It keeps parser signatures readable.
pub(crate) type AnnotationArgument<Identifier> = Argument<AnnotationLiteral<Identifier>>;

/// An intermediate annotation call.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct AnnotationCall<Identifier> {
	/// Annotation identifier.
	pub(crate) id: Identifier,
	/// Annotation arguments.
	pub(crate) args: Vec<AnnotationArgument<Identifier>>,
}

/// An intermediate annotation literal.
///
/// These are the same as regular [`Literal`]s, except that they may
/// additionally be a nested annotation.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum AnnotationLiteral<Identifier> {
	/// A regular literal value.
	Literal(Literal),
	/// Nested annotation value.
	Annotation(AnnotationCall<Identifier>),
}

/// An intermediate argument.
///
/// The literal type `L` is [`Literal`] for constraint arguments and
/// [`AnnotationLiteral`] for annotation arguments.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Argument<L = Literal> {
	/// An array of literals.
	Array(Vec<L>),
	/// A literal value.
	Literal(L),
}

/// An intermediate array declaration.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Array<Identifier> {
	/// Array contents.
	pub(crate) contents: Vec<Literal>,
	/// Array annotations.
	pub(crate) ann: Vec<Annotation<Identifier>>,
	/// Whether the array is defined by a constraint.
	pub(crate) defined: bool,
	/// Whether the array was introduced by MiniZinc lowering.
	pub(crate) introduced: bool,
}

/// An intermediate constraint.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Constraint<Identifier> {
	/// Constraint predicate identifier.
	pub(crate) id: Identifier,
	/// Constraint arguments.
	pub(crate) args: Vec<Argument>,
	/// Optional identifier of the defined variable.
	pub(crate) defines: Option<NameId>,
	/// Constraint annotations.
	pub(crate) ann: Vec<Annotation<Identifier>>,
}

/// Declaration state of a model namespace slot.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Declaration<Identifier> {
	/// Name has been referenced but not yet declared.
	Uninit,
	/// Name is declared as a variable.
	Variable(Variable<Identifier>),
	/// Name is declared as an array.
	Array(Array<Identifier>),
}

/// An intermediate FlatZinc model before identifiers have been linked to shared
/// definitions.
#[derive(PartialEq, Debug)]
pub(crate) struct FlatZinc<Identifier> {
	/// Interned store for all variable and array names referenced by the model.
	pub(crate) names: NameStore<Identifier>,
	/// Solver constraints in source order.
	pub(crate) constraints: Vec<Constraint<Identifier>>,
	/// Output targets collected from declarations or JSON output arrays.
	pub(crate) output: Vec<NameId>,
	/// Solve objective.
	pub(crate) solve: SolveObjective<Identifier>,
	/// Serialized FlatZinc version string.
	pub(crate) version: String,
}

/// Internal linker state used by [`TryFrom<FlatZinc<_>>`].
struct Linker<Identifier, F, Ref: FznRef = Immutable> {
	/// Interned model namespace and intermediate declarations.
	names: NameStore<Identifier>,
	/// Identifier interner function.
	interner: F,
	/// Intermediate constraints in source order.
	constraints: Vec<Constraint<Identifier>>,
	/// Intermediate output references.
	output: Vec<NameId>,
	/// Intermediate solve objective.
	solve: SolveObjective<Identifier>,
	/// FlatZinc serialization version.
	version: String,
	/// Cache of resulting literals when resolving a [`NameId`].
	resolved: Vec<Option<crate::Argument<Identifier, Ref>>>,
	/// Marks each [`NameId`] whose declaration is currently being constructed,
	/// so that a reference back to it can be recognized as self/cyclic.
	in_progress: Vec<bool>,
	/// Set while linking a single top-level annotation when it (transitively)
	/// references a name that is still under construction. Such annotations are
	/// silently dropped instead of becoming atoms or forming reference cycles.
	self_ref: bool,
}

/// An intermediate literal.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Literal {
	/// Integer value.
	Int(i64),
	/// Floating-point value.
	Float(f64),
	/// Identifier that must link to a variable or array.
	Reference(NameId),
	/// Boolean value.
	Bool(bool),
	/// Integer set.
	IntSet(RangeList<i64>),
	/// Floating-point set.
	FloatSet(RangeList<f64>),
	/// String value.
	String(String),
}

/// An intermediate solve method.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Method {
	/// Find any satisfying solution.
	Satisfy,
	/// Minimize the given objective literal.
	Minimize(Literal),
	/// Maximize the given objective literal.
	Maximize(Literal),
}

/// Stored slot in the intermediate model namespace.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct NameEntry<Identifier> {
	/// Canonical model name text.
	pub(crate) name: Box<str>,
	/// Current declaration state for the slot.
	pub(crate) declaration: Declaration<Identifier>,
}

/// Compact identifier used for model-level names such as variables and arrays.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct NameId(usize);

/// Single namespace for all variable and array names seen during parsing.
#[derive(Debug)]
pub(crate) struct NameStore<Identifier> {
	/// Canonical slot storage for every model name.
	entries: Vec<NameEntry<Identifier>>,
	/// Name lookup table keyed by references into [`NameStore::entries`].
	lookup: HashMap<&'static str, NameId>,
}

/// Mutable parse-time state shared by input frontends while building the
/// intermediate model.
pub(crate) struct ParserState<Identifier, F> {
	/// Interned store for model-level names.
	pub(crate) names: NameStore<Identifier>,
	/// Caller-provided predicate/annotation identifier interner.
	pub(crate) interner: F,
	/// Deferred identifier-interning failure captured during parser-driven
	/// parsing.
	#[cfg(feature = "fzn")]
	pub(crate) identifier_error: Option<LinkError>,
}

/// An intermediate solve objective.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct SolveObjective<Identifier> {
	/// Solve method.
	pub(crate) method: Method,
	/// Solve annotations.
	pub(crate) ann: Vec<Annotation<Identifier>>,
}

/// An intermediate decision variable declaration.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Variable<Identifier> {
	/// Variable domain/type information.
	pub(crate) ty: Type,
	/// Variable annotations.
	pub(crate) ann: Vec<Annotation<Identifier>>,
	/// Whether the variable is defined by a constraint.
	pub(crate) defined: bool,
	/// Whether the variable was introduced by MiniZinc lowering.
	pub(crate) introduced: bool,
}

impl<Identifier> From<Literal> for AnnotationLiteral<Identifier> {
	fn from(value: Literal) -> Self {
		AnnotationLiteral::Literal(value)
	}
}

impl<Identifier, F, E, Ref: FznRef> Linker<Identifier, F, Ref>
where
	Identifier: Clone,
	F: FnMut(&str) -> Result<Identifier, E>,
	E: Display,
{
	/// Create a representation for a named array.
	fn create_array(
		&mut self,
		name: Box<str>,
		array: Array<Identifier>,
	) -> Result<crate::Argument<Identifier, Ref>, LinkError> {
		let array = Ref::new(crate::Array {
			name: name.into_string(),
			contents: self.link_literals(array.contents)?,
			ann: self.link_annotations(array.ann)?,
			defined: array.defined,
			introduced: array.introduced,
		});
		Ok(crate::Argument::ArrayNamed(array))
	}

	/// Create a representation for a decision variable
	fn create_variable(
		&mut self,
		name: Box<str>,
		variable: Variable<Identifier>,
	) -> Result<crate::Literal<Identifier, Ref>, LinkError> {
		Ok(crate::Literal::Variable(Ref::new(crate::Variable {
			name: name.into_string(),
			ty: variable.ty,
			ann: self.link_annotations(variable.ann)?,
			defined: variable.defined,
			introduced: variable.introduced,
		})))
	}

	/// Convert the owned intermediate model into the public FlatZinc graph.
	fn link(mut self) -> Result<crate::FlatZinc<Identifier, Ref>, LinkError> {
		let mut var_names = Vec::new();
		let mut arr_names = Vec::new();

		for (name, entry) in self.names.iter() {
			match &entry.declaration {
				Declaration::Variable(_) => var_names.push(name),
				Declaration::Array(_) => arr_names.push(name),
				_ => {}
			}
		}
		let variables = var_names
			.into_iter()
			.filter_map(|name| {
				self.link_name(name).transpose().map(|r| {
					r.map(|r| {
						let NamedRef::Variable(v) = r else {
							unreachable!()
						};
						v
					})
				})
			})
			.collect::<Result<Vec<_>, _>>()?;

		let arrays = arr_names
			.into_iter()
			.filter_map(|name| {
				self.link_name(name).transpose().map(|r| {
					r.map(|r| {
						let NamedRef::Array(v) = r else {
							unreachable!()
						};
						v
					})
				})
			})
			.collect::<Result<Vec<_>, _>>()?;

		let constraints = mem::take(&mut self.constraints);
		let constraints = constraints
			.into_iter()
			.map(|constraint| self.link_constraint(constraint))
			.collect::<Result<Vec<_>, _>>()?;

		let output = mem::take(&mut self.output);
		let output = output
			.into_iter()
			.flat_map(|name| self.link_name(name).transpose())
			.collect::<Result<Vec<_>, _>>()?;

		let ann = mem::take(&mut self.solve.ann);
		let solve = crate::SolveObjective {
			method: match self.solve.method.clone() {
				Method::Satisfy => crate::Method::Satisfy,
				Method::Minimize(lit) => crate::Method::Minimize(self.link_literal(lit)?),
				Method::Maximize(lit) => crate::Method::Maximize(self.link_literal(lit)?),
			},
			ann: self.link_annotations(ann)?,
		};

		Ok(crate::FlatZinc {
			variables,
			arrays,
			constraints,
			output,
			solve,
			version: self.version,
		})
	}

	/// Link one intermediate annotation.
	fn link_annotation(
		&mut self,
		annotation: Annotation<Identifier>,
	) -> Result<crate::Annotation<Identifier, Ref>, LinkError> {
		match annotation {
			Annotation::Atom(id) => Ok(crate::Annotation::Atom(id)),
			Annotation::Call(call) => Ok(crate::Annotation::Call(self.link_annotation_call(call)?)),
		}
	}

	/// Link one intermediate annotation argument.
	fn link_annotation_argument(
		&mut self,
		arg: AnnotationArgument<Identifier>,
	) -> Result<
		crate::Argument<Identifier, Ref, crate::AnnotationLiteral<Identifier, Ref>>,
		LinkError,
	> {
		match arg {
			Argument::Array(values) => Ok(crate::Argument::<_, _, _>::Array(
				values
					.into_iter()
					.map(|value| self.link_annotation_literal(value))
					.collect::<Result<Vec<_>, _>>()?,
			)),
			Argument::Literal(AnnotationLiteral::Literal(Literal::Reference(name))) => {
				match self.link_argument(Argument::Literal(Literal::Reference(name))) {
					Ok(arg) => Ok(arg.into()),
					Err(err @ LinkError::UnknownReference(_)) if self.self_ref => Err(err),
					Err(LinkError::UnknownReference(_)) => {
						debug_assert!(matches!(
							self.names.entries[name.index()].declaration,
							Declaration::Uninit
						));
						let ident = (self.interner)(self.names.entries[name.index()].name.as_ref())
							.map_err(|err| LinkError::IdentifierError {
								ident: self.names.to_owned(name),
								err: err.to_string(),
							})?;
						Ok(crate::Argument::<_, _, _>::Literal(
							crate::AnnotationLiteral::Annotation(crate::Annotation::Atom(ident)),
						))
					}
					Err(err) => Err(err),
				}
			}
			Argument::Literal(value) => Ok(crate::Argument::<_, _, _>::Literal(
				self.link_annotation_literal(value)?,
			)),
		}
	}

	/// Link one intermediate annotation call.
	fn link_annotation_call(
		&mut self,
		call: AnnotationCall<Identifier>,
	) -> Result<crate::AnnotationCall<Identifier, Ref>, LinkError> {
		Ok(crate::AnnotationCall {
			id: call.id,
			args: call
				.args
				.into_iter()
				.map(|arg| self.link_annotation_argument(arg))
				.collect::<Result<Vec<_>, _>>()?,
		})
	}

	/// Link one intermediate annotation literal.
	fn link_annotation_literal(
		&mut self,
		literal: AnnotationLiteral<Identifier>,
	) -> Result<crate::AnnotationLiteral<Identifier, Ref>, LinkError> {
		Ok(match literal {
			AnnotationLiteral::Literal(Literal::Reference(name)) => {
				match self.link_literal(Literal::Reference(name)) {
					Ok(lit) => lit.into(),
					Err(err @ LinkError::UnknownReference(_)) if self.self_ref => return Err(err),
					Err(LinkError::UnknownReference(_)) => {
						debug_assert!(matches!(
							self.names.entries[name.index()].declaration,
							Declaration::Uninit
						));
						let ident = (self.interner)(self.names.entries[name.index()].name.as_ref())
							.map_err(|err| LinkError::IdentifierError {
								ident: self.names.to_owned(name),
								err: err.to_string(),
							})?;
						crate::AnnotationLiteral::Annotation(crate::Annotation::Atom(ident))
					}
					Err(err) => return Err(err),
				}
			}
			AnnotationLiteral::Literal(lit) => self.link_literal(lit)?.into(),
			AnnotationLiteral::Annotation(annotation) => crate::AnnotationLiteral::Annotation(
				crate::Annotation::Call(self.link_annotation_call(annotation)?),
			),
		})
	}

	/// Link a list of intermediate annotations, silently dropping any
	/// annotation that references a name still under construction (a
	/// self/cyclic reference).
	fn link_annotations(
		&mut self,
		annotations: Vec<Annotation<Identifier>>,
	) -> Result<Vec<crate::Annotation<Identifier, Ref>>, LinkError> {
		let mut linked = Vec::with_capacity(annotations.len());
		for annotation in annotations {
			self.self_ref = false;
			match self.link_annotation(annotation) {
				Ok(annotation) => linked.push(annotation),
				// A self/cyclic reference unwinds as an unknown reference with
				// `self_ref` set: drop this annotation rather than surfacing it.
				Err(_) if self.self_ref => {}
				Err(err) => return Err(err),
			}
		}
		Ok(linked)
	}

	/// Link one intermediate argument.
	fn link_argument(
		&mut self,
		arg: Argument,
	) -> Result<crate::Argument<Identifier, Ref>, LinkError> {
		Ok(match arg {
			Argument::Array(lits) => crate::Argument::Array(
				lits.into_iter()
					.map(|lit| self.link_literal(lit))
					.collect::<Result<Vec<_>, _>>()?,
			),
			Argument::Literal(Literal::Reference(name)) => self.resolve_name(name)?,
			Argument::Literal(lit) => crate::Argument::Literal(self.link_literal(lit)?),
		})
	}

	/// Link one intermediate constraint.
	fn link_constraint(
		&mut self,
		constraint: Constraint<Identifier>,
	) -> Result<crate::Constraint<Identifier, Ref>, LinkError> {
		let defines = if let Some(name) = constraint.defines {
			self.link_name(name)?
		} else {
			None
		};
		Ok(crate::Constraint {
			id: constraint.id,
			args: constraint
				.args
				.into_iter()
				.map(|arg| self.link_argument(arg))
				.collect::<Result<Vec<_>, _>>()?,
			defines,
			ann: self.link_annotations(constraint.ann)?,
		})
	}

	/// Link one intermediate literal.
	fn link_literal(
		&mut self,
		literal: Literal,
	) -> Result<crate::Literal<Identifier, Ref>, LinkError> {
		Ok(match literal {
			Literal::Int(i) => crate::Literal::Int(i),
			Literal::Float(f) => crate::Literal::Float(f),
			Literal::Reference(name) => self.resolve_literal_name(name)?,
			Literal::Bool(b) => crate::Literal::Bool(b),
			Literal::IntSet(ranges) => crate::Literal::IntSet(ranges),
			Literal::FloatSet(ranges) => crate::Literal::FloatSet(ranges),
			Literal::String(string) => crate::Literal::String(string),
		})
	}

	/// Link a list of intermediate literals.
	fn link_literals(
		&mut self,
		literals: Vec<Literal>,
	) -> Result<Vec<crate::Literal<Identifier, Ref>>, LinkError> {
		literals
			.into_iter()
			.map(|literal| self.link_literal(literal))
			.collect()
	}

	/// Link the named reference if possible, returning `None` if the name does
	/// not result in a variable or named array in the resulting model.
	fn link_name(&mut self, name: NameId) -> Result<Option<NamedRef<Identifier, Ref>>, LinkError> {
		Ok(match self.resolve_name(name)? {
			crate::Argument::Literal(crate::Literal::Variable(var)) => {
				Some(NamedRef::Variable(var))
			}
			crate::Argument::ArrayNamed(arr) => Some(NamedRef::Array(arr)),
			crate::Argument::Literal(_) | crate::Argument::Array(_) => None,
		})
	}

	/// Create linker state from an intermediate model.
	fn new(model: FlatZinc<Identifier>, interner: F) -> Self {
		let names_len = model.names.len();
		Self {
			names: model.names,
			interner,
			constraints: model.constraints,
			output: model.output,
			solve: model.solve,
			version: model.version,
			resolved: vec![None; names_len],
			in_progress: vec![false; names_len],
			self_ref: false,
		}
	}

	/// Resolve one model-level name into a linked literal.
	fn resolve_literal_name(
		&mut self,
		name: NameId,
	) -> Result<crate::Literal<Identifier, Ref>, LinkError> {
		match self.resolve_name(name)? {
			crate::Argument::Literal(lit) => Ok(lit),
			crate::Argument::ArrayNamed(_) => {
				Err(LinkError::NestedArray(self.names.to_owned(name)))
			}
			crate::Argument::Array(_) => unreachable!("resolved names never produce inline arrays"),
		}
	}

	/// Resolve one model-level name into its linked argument form.
	fn resolve_name(
		&mut self,
		name: NameId,
	) -> Result<crate::Argument<Identifier, Ref>, LinkError> {
		if let Some(arg) = &self.resolved[name.index()] {
			return Ok(arg.clone());
		}
		if self.in_progress[name.index()] {
			// Back-edge to a name whose annotations are still being linked: the
			// enclosing annotation is self/cyclic and gets dropped by
			// `link_annotations`. Reported as an unknown reference so it unwinds
			// through the normal machinery without extracting the entry.
			self.self_ref = true;
			return Err(LinkError::UnknownReference(self.names.to_owned(name)));
		}
		let entry = self.names.extract_entry(name);
		let arg = match entry.declaration {
			Declaration::Variable(variable) => {
				self.in_progress[name.index()] = true;
				// Isolate the caller's annotation from self-references dropped
				// while constructing this (nested) variable's own annotations.
				let saved = self.self_ref;
				let result = self.create_variable(entry.name, variable);
				self.self_ref = saved;
				crate::Argument::Literal(result?)
			}
			Declaration::Array(array) => {
				self.in_progress[name.index()] = true;
				let saved = self.self_ref;
				let result = self.create_array(entry.name, array);
				self.self_ref = saved;
				result?
			}
			Declaration::Uninit => {
				self.names.entries[name.index()] = entry;
				return Err(LinkError::UnknownReference(self.names.to_owned(name)));
			}
		};
		self.resolved[name.index()] = Some(arg.clone());
		Ok(arg)
	}
}

impl NameId {
	/// Return the backing table index.
	pub(crate) fn index(self) -> usize {
		self.0
	}

	/// Create a name identifier from a raw table index.
	pub(crate) fn new(index: usize) -> Self {
		Self(index)
	}
}

impl<Identifier> NameStore<Identifier> {
	/// Define the slot using the given declaration.
	fn define_name(&mut self, id: NameId, decl: Declaration<Identifier>) -> Result<(), LinkError> {
		match self.entries[id.index()].declaration {
			Declaration::Uninit => {
				self.entries[id.index()].declaration = decl;
				Ok(())
			}
			_ => Err(LinkError::DuplicateDefinition(self.to_owned(id))),
		}
	}

	/// Remove the stored entry for a model name.
	pub(crate) fn extract_entry(&mut self, id: NameId) -> NameEntry<Identifier> {
		mem::replace(
			&mut self.entries[id.index()],
			NameEntry {
				name: Box::default(),
				declaration: Declaration::Uninit,
			},
		)
	}

	/// Intern a model name and return its compact identifier.
	pub(crate) fn intern(&mut self, name: &str) -> NameId {
		if let Some(&id) = self.lookup.get(name) {
			return id;
		}

		let id = NameId::new(self.entries.len());
		self.entries.push(NameEntry {
			name: name.into(),
			declaration: Declaration::Uninit,
		});
		// SAFETY: The `name` string is put into a static lookup table key,
		// using is safe because the string is immutable and owned by the
		// `NameStore`, but should never be leaked to outside `NameStore`
		// boundaries.
		let key =
			unsafe { mem::transmute::<&str, &'static str>(self.entries[id.index()].name.as_ref()) };
		let _ = self.lookup.insert(key, id);
		id
	}

	/// Iterate over all name entries in insertion order.
	pub(crate) fn iter(&self) -> impl Iterator<Item = (NameId, &NameEntry<Identifier>)> {
		self.entries
			.iter()
			.enumerate()
			.map(|(index, entry)| (NameId::new(index), entry))
	}

	/// Return the number of interned model names.
	pub(crate) fn len(&self) -> usize {
		self.entries.len()
	}

	/// Resolve a compact identifier back to the stored model name.
	pub(crate) fn resolve(&self, id: NameId) -> &str {
		&self.entries[id.index()].name
	}

	/// Clone the stored model name for transfer into the public AST.
	pub(crate) fn to_owned(&self, id: NameId) -> String {
		self.resolve(id).to_owned()
	}
}

impl<Identifier> Default for NameStore<Identifier> {
	/// Create an empty model-name store.
	fn default() -> Self {
		Self {
			entries: Vec::new(),
			lookup: HashMap::new(),
		}
	}
}

impl<Identifier: PartialEq> PartialEq for NameStore<Identifier> {
	/// Compare stores by their canonical entry contents.
	fn eq(&self, other: &Self) -> bool {
		self.entries == other.entries
	}
}

impl<Identifier, F> ParserState<Identifier, F> {
	/// Define one array declaration in the intermediate namespace.
	#[cfg(feature = "serde")]
	pub(crate) fn define_array(
		&mut self,
		name: NameId,
		array: Array<Identifier>,
	) -> Result<(), LinkError> {
		self.define_name(name, Declaration::Array(array))
	}

	/// Define one variable declaration in the intermediate namespace.
	pub(crate) fn define_name(
		&mut self,
		name: NameId,
		decl: Declaration<Identifier>,
	) -> Result<(), LinkError> {
		self.names.define_name(name, decl)
	}

	/// Define one variable declaration in the intermediate namespace.
	#[cfg(feature = "serde")]
	pub(crate) fn define_variable(
		&mut self,
		name: NameId,
		variable: Variable<Identifier>,
	) -> Result<(), LinkError> {
		self.define_name(name, Declaration::Variable(variable))
	}

	/// Intern one model-level name and return its compact identifier.
	#[cfg(feature = "fzn")]
	pub(crate) fn intern_name(&mut self, name: &str) -> NameId {
		self.names.intern(name)
	}

	/// Consume the parser state and return the owned name store and interner.
	pub(crate) fn into_parts(self) -> (NameStore<Identifier>, F) {
		(self.names, self.interner)
	}

	/// Create parser state backed by the caller-provided identifier interner.
	pub(crate) fn new(interner: F) -> Self {
		Self {
			names: NameStore::default(),
			interner,
			#[cfg(feature = "fzn")]
			identifier_error: None,
		}
	}

	/// Record an identifier-interning failure for later reporting.
	#[cfg(feature = "fzn")]
	pub(crate) fn record_identifier_error(&mut self, ident: &str, err: String) {
		self.identifier_error = Some(LinkError::IdentifierError {
			ident: ident.to_owned(),
			err,
		});
	}

	/// Extract the deferred identifier-interning failure, if any.
	#[cfg(feature = "fzn")]
	pub(crate) fn take_identifier_error(&mut self) -> Option<LinkError> {
		self.identifier_error.take()
	}
}

impl<Identifier, F> ParserState<Identifier, F>
where
	Identifier: Clone,
{
	/// Intern a predicate or annotation identifier.
	pub(crate) fn intern_identifier<E>(&mut self, ident: &str) -> Result<Identifier, String>
	where
		F: FnMut(&str) -> Result<Identifier, E>,
		E: Display,
	{
		(self.interner)(ident)
			.map_err(|err| format!("failed to intern identifier `{ident}`: {err}"))
	}
}

impl<Identifier, F> Debug for ParserState<Identifier, F> {
	/// Format parser state without requiring debug support from the caller's
	/// identifier or interner types.
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ParserState").finish_non_exhaustive()
	}
}

impl<Identifier: Clone, Ref: FznRef> crate::FlatZinc<Identifier, Ref> {
	/// Convert an intermediate [`FlatZinc`] model into a [`crate::FlatZinc`]
	/// instance, finalizing annotation atoms using the provided interner.
	pub(crate) fn from_intermediate<F, E>(
		model: FlatZinc<Identifier>,
		interner: F,
	) -> Result<Self, LinkError>
	where
		F: FnMut(&str) -> Result<Identifier, E>,
		E: Display,
	{
		Linker::new(model, interner).link()
	}
}
