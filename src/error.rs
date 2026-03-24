//! Error types for FlatZinc parsing or deserialization.

use std::{
	error::Error,
	fmt::{self, Display},
	io,
	str::Utf8Error,
};

#[cfg(feature = "fzn")]
use winnow::error::{ContextError, ParseError};

#[cfg(feature = "fzn")]
use crate::fzn::Stream;

/// Errors that can occur when parsing `.fzn` models.
#[derive(Debug)]
pub enum FznParseError {
	/// Error reading from the source.
	Io(io::Error),
	/// Error converting to utf8.
	Utf8Error(Utf8Error),
	/// Missing solve item in the model.
	MissingSolveItem,
	/// Multiple solve items were encountered in the model.
	MultipleSolveItems,
	/// An error in the syntax of the `fzn`.
	SyntaxError(String),
	/// An error when linking the variables and arrays in the model.
	LinkError(LinkError),
}

/// Error produced while linking an variable and array references of the model.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LinkError {
	/// A predicate or annotation identifier could not be interned.
	IdentifierError {
		/// The identifier string that failed to intern.
		ident: String,
		/// The interner error message.
		err: String,
	},
	/// A named literal could not be resolved to a variable or array.
	UnknownReference(String),
	/// A named array identifier that was used as a literal.
	NestedArray(String),
	/// Duplicate definition of a name.
	DuplicateDefinition(String),
}

impl Display for FznParseError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Io(error) => write!(f, "error reading from source: {error}"),
			Self::Utf8Error(error) => write!(f, "invalid utf8: {error}"),
			Self::MissingSolveItem => write!(f, "missing solve item"),
			Self::MultipleSolveItems => write!(f, "multiple solve items"),
			Self::SyntaxError(error) => write!(f, "syntax error: {error}"),
			Self::LinkError(error) => {
				write!(f, "error linking model: {error}")
			}
		}
	}
}

impl Error for FznParseError {}

impl From<LinkError> for FznParseError {
	fn from(error: LinkError) -> Self {
		Self::LinkError(error)
	}
}

#[cfg(feature = "fzn")]
impl<Identifier, F> From<ParseError<Stream<'_, Identifier, F>, ContextError>> for FznParseError {
	fn from(value: ParseError<Stream<'_, Identifier, F>, ContextError>) -> Self {
		Self::SyntaxError(value.to_string())
	}
}

impl From<Utf8Error> for FznParseError {
	fn from(value: Utf8Error) -> Self {
		Self::Utf8Error(value)
	}
}

impl From<io::Error> for FznParseError {
	fn from(value: io::Error) -> Self {
		Self::Io(value)
	}
}

impl Display for LinkError {
	/// Format a linker error for parser and deserializer diagnostics.
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::IdentifierError { ident, err } => {
				write!(f, "failed to intern identifier `{ident}`: {err}")
			}
			Self::UnknownReference(name) => {
				write!(f, "unknown FlatZinc variable or array `{name}`")
			}
			Self::NestedArray(name) => {
				write!(f, "array identifier `{name}` used as part of an array")
			}
			Self::DuplicateDefinition(name) => {
				write!(f, "duplicate definition of `{name}`")
			}
		}
	}
}

impl Error for LinkError {}
