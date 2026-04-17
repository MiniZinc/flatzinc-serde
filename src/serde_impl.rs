//! Helper structures and functions to parse and output using `serde` certain
//! types in the FlatZinc JSON serialization

pub(crate) mod seeded;

use std::{
	fmt::{Debug, Display},
	ops::Deref,
	sync::{Arc, Weak},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};

use crate::{
	Annotation, Array, FlatZinc, Literal, Method, NamedRef, RangeList, SolveObjective, Type,
	Variable,
};

/// Base variable type used for the `"type"` field in FlatZinc JSON.
///
/// This mirrors the JSON encoding, where the type name and optional domain are
/// serialized as separate fields.
#[derive(Clone, Copy, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename = "type")]
pub(crate) enum BaseType {
	/// Boolean decision variable type encoded as `"bool"`.
	#[serde(rename = "bool")]
	Bool,
	/// Integer decision variable type encoded as `"int"`.
	#[serde(rename = "int")]
	Int,
	/// Floating-point decision variable type encoded as `"float"`.
	#[serde(rename = "float")]
	Float,
	/// Integer set decision variable type encoded as `"set of int"`.
	#[serde(rename = "set of int")]
	IntSet,
}

/// Domain payload used for the optional `"domain"` field in FlatZinc JSON.
///
/// This is kept separate from [`BaseType`] so [`crate::Variable`] can preserve
/// the historical JSON shape while internally storing domains inside [`Type`].
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum VariableDomain {
	/// Integer domain payload serialized as a JSON array of inclusive bounds.
	#[serde(deserialize_with = "deserialize_set", serialize_with = "serialize_set")]
	Int(RangeList<i64>),
	/// Floating-point domain payload serialized as a JSON array of inclusive
	/// bounds.
	#[serde(deserialize_with = "deserialize_set", serialize_with = "serialize_set")]
	Float(RangeList<f64>),
}

/// Deserialize a raw `(start, end)` range list into a [`RangeList`].
pub(crate) fn deserialize_set<
	'de,
	D: Deserializer<'de>,
	E: Copy + Deserialize<'de> + PartialOrd + 'static,
>(
	deserializer: D,
) -> Result<RangeList<E>, D::Error> {
	let s: Vec<(E, E)> = Deserialize::deserialize(deserializer)?;
	let range = s.into_iter().map(|(a, b)| a..=b).collect();
	Ok(range)
}

/// Helper function used by serde field attributes to omit `false` flags.
pub(crate) fn is_false(b: &bool) -> bool {
	!(*b)
}

/// Serialize an [`Arc<Array>`] by serializing its name, if present, or its
/// contents otherwise.
pub(crate) fn serialize_array_arc<S: Serializer, Identifier>(
	array: &Arc<Array<Identifier>>,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	Serialize::serialize(&array.name, serializer)
}

/// Serialize a slice of [`Arc<Array>`] references into a key-value map, using
/// the array name as the key.
pub(crate) fn serialize_array_map<S: Serializer, Identifier: Serialize>(
	arrays: &[Arc<Array<Identifier>>],
	serializer: S,
) -> Result<S::Ok, S::Error> {
	let mut state = serializer.serialize_map(Some(arrays.len()))?;
	for arr in arrays {
		state.serialize_entry(&arr.name, arr.deref())?;
	}
	state.end()
}

/// Serialize a [`Weak<Array>`] reference, upgrading it to an [`Arc<Array>`]
/// before serializing it using its name, if present, or its contents otherwise.
pub(crate) fn serialize_array_weak<S: Serializer, Identifier>(
	array: &Weak<Array<Identifier>>,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	let array = array.upgrade().ok_or_else(|| {
		serde::ser::Error::custom("dangling weak array reference in annotation literal")
	})?;
	serialize_array_arc(&array, serializer)
}

/// Serialization function to be used for the encapsulation of set literals
/// required by the FlatZinc serialization format
pub(crate) fn serialize_encapsulate_set<E: PartialOrd + Serialize + Copy, S: Serializer>(
	r: &RangeList<E>,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	#[derive(Serialize)]
	struct SetLiteral<E: PartialOrd> {
		set: Vec<(E, E)>,
	}

	Serialize::serialize(
		&SetLiteral {
			set: r.iter().map(|r| (*r.start(), *r.end())).collect(),
		},
		serializer,
	)
}

/// Serialization function to be used for the encapsulation of string literals
/// required by the FlatZinc serialization format
pub(crate) fn serialize_encapsulate_string<S: Serializer>(
	s: &str,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	#[derive(Serialize)]
	struct StringLiteral<'a> {
		string: &'a str,
	}

	Serialize::serialize(&StringLiteral { string: s }, serializer)
}

/// Serialization function to be used for the encapsulation of set literals
/// required by the FlatZinc serialization format
pub(crate) fn serialize_set<E: PartialOrd + Serialize + Copy, S: Serializer>(
	r: &RangeList<E>,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	let x: Vec<(E, E)> = r.iter().map(|r| (*r.start(), *r.end())).collect();
	Serialize::serialize(&x, serializer)
}

/// Serialize an [`Arc<Variable>`] by serializing its name.
pub(crate) fn serialize_variable_arc<S: Serializer, Identifier>(
	variable: &Arc<Variable<Identifier>>,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	Serialize::serialize(&variable.name, serializer)
}

/// Serialize a slice of variables into a key-value map, using the variable
/// name as the key.
pub(crate) fn serialize_variable_map<S: Serializer, Identifier: Serialize>(
	variables: &[Arc<Variable<Identifier>>],
	serializer: S,
) -> Result<S::Ok, S::Error> {
	let mut state = serializer.serialize_map(Some(variables.len()))?;
	for var in variables {
		state.serialize_entry(&var.name, var.deref())?;
	}
	state.end()
}

/// Serialize a [`Weak<Variable>`] reference, upgrading it to an
/// [`Arc<Variable>`] before serializing it using its name.
pub(crate) fn serialize_variable_weak<S: Serializer, Identifier>(
	variable: &Weak<Variable<Identifier>>,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	let variable = variable.upgrade().ok_or_else(|| {
		serde::ser::Error::custom("dangling weak variable reference in annotation literal")
	})?;
	serialize_variable_arc(&variable, serializer)
}

impl<'de, I, E> Deserialize<'de> for FlatZinc<I>
where
	I: Clone + Debug + for<'a> TryFrom<&'a str, Error = E>,
	E: Display,
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		Self::deserialize_with_interner(deserializer, |ident| I::try_from(ident))
	}
}

impl<Identifier: Serialize> Serialize for NamedRef<Identifier> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		self.name().serialize(serializer)
	}
}

impl<Identifier: Serialize> Serialize for SolveObjective<Identifier> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		#[derive(Serialize)]
		struct SolveObjectiveRepr<'a, Identifier> {
			method: &'static str,
			#[serde(skip_serializing_if = "Option::is_none")]
			objective: Option<&'a Literal<Identifier>>,
			#[serde(skip_serializing_if = "Vec::is_empty")]
			ann: &'a Vec<Annotation<Identifier>>,
		}

		let (method, objective) = match &self.method {
			Method::Satisfy => ("satisfy", None),
			Method::Minimize(objective) => ("minimize", Some(objective)),
			Method::Maximize(objective) => ("maximize", Some(objective)),
		};

		SolveObjectiveRepr {
			method,
			objective,
			ann: &self.ann,
		}
		.serialize(serializer)
	}
}

impl<Identifier: Serialize> Serialize for Variable<Identifier> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		#[derive(Serialize)]
		struct VariableRepr<'a, Identifier> {
			#[serde(rename = "type")]
			ty: BaseType,
			#[serde(skip_serializing_if = "Option::is_none")]
			domain: Option<VariableDomain>,
			#[serde(default, skip_serializing_if = "Vec::is_empty")]
			ann: &'a Vec<Annotation<Identifier>>,
			#[serde(default, skip_serializing_if = "is_false")]
			defined: bool,
			#[serde(default, skip_serializing_if = "is_false")]
			introduced: bool,
		}

		let (ty, domain) = match &self.ty {
			Type::Bool => (BaseType::Bool, None),
			Type::Int(domain) => (BaseType::Int, domain.clone().map(VariableDomain::Int)),
			Type::Float(domain) => (BaseType::Float, domain.clone().map(VariableDomain::Float)),
			Type::IntSet(domain) => (BaseType::IntSet, domain.clone().map(VariableDomain::Int)),
		};

		VariableRepr {
			ty,
			domain,
			ann: &self.ann,
			defined: self.defined,
			introduced: self.introduced,
		}
		.serialize(serializer)
	}
}

#[cfg(test)]
mod tests {
	macro_rules! test_file {
		($file: ident) => {
			#[test]
			fn $file() {
				test_successful_serialization(
					std::path::Path::new(&format!("./corpus/json/{}.fzn.json", stringify!($file))),
					expect_test::expect_file![&format!(
						"../corpus/json/{}.debug.txt",
						stringify!($file)
					)],
				)
			}
		};
	}

	use std::{
		convert::Infallible,
		fs::File,
		io::{BufReader, Read},
		path::Path,
		sync::Arc,
	};

	use expect_test::ExpectFile;
	use rangelist::RangeList;
	use serde_json::{Deserializer, json};
	use ustr::Ustr;

	use crate::{
		Annotation, AnnotationArgument, AnnotationCall, AnnotationLiteral, Argument, Array,
		FlatZinc, Literal, Method, NamedRef, SolveObjective, Type, Variable,
	};

	#[test]
	fn test_default_flatzinc() {
		let fzn = FlatZinc::<String>::default();
		assert!(fzn.variables.is_empty());
		assert!(fzn.arrays.is_empty());
		assert!(fzn.constraints.is_empty());
		assert!(fzn.output.is_empty());
		assert_eq!(fzn.version, "1.0");
	}

	#[test]
	fn test_deserialize_with_custom_interner() {
		#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
		struct Interned(String);

		let json = r#"{
			"variables": {
				"x": {"type": "int"}
			},
			"constraints": [
				{"id": "int_eq", "args": ["x", 1]}
			],
			"solve": {"method": "satisfy"},
			"output": ["x"],
			"version": "1.0"
		}"#;

		let mut de = Deserializer::from_str(json);
		let fzn = FlatZinc::<Interned>::deserialize_with_interner(&mut de, |s| {
			Ok::<_, Infallible>(Interned(format!("intern:{s}")))
		})
		.unwrap();

		assert_eq!(
			fzn.output.iter().map(NamedRef::name).collect::<Vec<_>>(),
			vec!["x"]
		);
		assert_eq!(fzn.constraints[0].id, Interned("intern:int_eq".to_owned()));
		assert_eq!(fzn.variables.len(), 1);
		let Argument::Literal(Literal::Variable(variable)) = &fzn.constraints[0].args[0] else {
			unreachable!()
		};
		assert!(Arc::ptr_eq(variable, &fzn.variables[0]));
		assert_eq!(
			fzn.constraints[0].args[1],
			Argument::Literal(Literal::Int(1))
		);
	}

	#[test]
	fn test_deserialize_with_interner_rejects_variable_rhs() {
		let json = r#"{
			"variables": {
				"y": {"type": "int", "rhs": 5}
			},
			"constraints": [],
			"solve": {"method": "satisfy"},
			"version": "1.0"
		}"#;

		let err =
			FlatZinc::<String>::deserialize_with_interner(&mut Deserializer::from_str(json), |s| {
				Ok::<_, Infallible>(s.into())
			})
			.expect_err("expected variable rhs to be rejected");

		assert!(err.to_string().contains("unknown field `rhs`"));
	}

	#[test]
	fn test_ident_interned() {
		let rdr = BufReader::new(
			File::open(Path::new("./corpus/json/documentation_example.fzn.json")).unwrap(),
		);
		let fzn: FlatZinc<Ustr> = serde_json::from_reader(rdr).unwrap();
		expect_test::expect_file!["../corpus/json/documentation_example.debug_ustr.txt"]
			.assert_debug_eq(&fzn)
	}

	#[test]
	fn test_json_shape_preserves_named_objects() {
		let x = Arc::new(Variable::<String> {
			name: "x".to_owned(),
			ty: Type::Int(None),
			ann: vec![],
			defined: false,
			introduced: false,
		});
		let y = Arc::new(Array::<String> {
			name: "y".to_owned(),
			contents: vec![Literal::Int(1), Literal::Variable(Arc::clone(&x))],
			ann: vec![],
			defined: false,
			introduced: false,
		});
		let fzn = FlatZinc {
			variables: vec![Arc::clone(&x)],
			arrays: vec![Arc::clone(&y)],
			constraints: vec![],
			output: vec![
				NamedRef::Variable(Arc::clone(&x)),
				NamedRef::Array(Arc::clone(&y)),
			],
			solve: SolveObjective {
				method: Method::Satisfy,
				ann: vec![],
			},
			version: "1.0".to_owned(),
		};

		let value = serde_json::to_value(&fzn).unwrap();
		assert_eq!(
			value,
			json!({
				"variables": {
					"x": {"type": "int"}
				},
				"arrays": {
					"y": {"a": [1, "x"]}
				},
				"constraints": [],
				"output": ["x", "y"],
				"solve": {"method": "satisfy"},
				"version": "1.0"
			})
		);
	}

	#[test]
	fn test_print_flatzinc() {
		let mut rdr = BufReader::new(
			File::open(Path::new("./corpus/json/documentation_example.fzn.json")).unwrap(),
		);
		let mut content = String::new();
		let _ = rdr.read_to_string(&mut content).unwrap();

		let fzn: FlatZinc = serde_json::from_str(&content).unwrap();
		expect_test::expect_file!["../corpus/fzn/documentation_example.fzn"]
			.assert_eq(&fzn.to_string());

		let ann: Annotation<&str> = Annotation::Call(AnnotationCall {
			id: "bool_search",
			args: vec![
				AnnotationArgument::Literal(AnnotationLiteral::Annotation(Annotation::Atom(
					"input_order",
				))),
				AnnotationArgument::Literal(AnnotationLiteral::Annotation(Annotation::Atom(
					"indomain_min",
				))),
			],
		});
		assert_eq!(ann.to_string(), "bool_search(input_order, indomain_min)");

		let ty = Type::Bool;
		assert_eq!(ty.to_string(), "bool");
		let ty = Type::Int(None);
		assert_eq!(ty.to_string(), "int");
		let ty = Type::Float(None);
		assert_eq!(ty.to_string(), "float");
		let ty = Type::IntSet(None);
		assert_eq!(ty.to_string(), "set of int");
		let ty = Type::Float(Some(RangeList::from(1.0..=4.0)));
		assert_eq!(ty.to_string(), "1.0..4.0");

		let lit = Literal::<&str>::Int(1);
		assert_eq!(lit.to_string(), "1");
		let lit = Literal::<&str>::Float(1.0);
		assert_eq!(lit.to_string(), "1.0");
		let x = Arc::new(Variable {
			name: "x".to_owned(),
			ty: Type::IntSet(None),
			ann: vec![Annotation::Atom("special")],
			defined: false,
			introduced: true,
		});
		let lit = Literal::Variable(Arc::clone(&x));
		assert_eq!(lit.to_string(), "x");
		let lit = Literal::<&str>::Bool(true);
		assert_eq!(lit.to_string(), "true");
		let lit = Literal::<&str>::IntSet(RangeList::from(2..=3));
		assert_eq!(lit.to_string(), "2..3");
		let lit = Literal::<&str>::FloatSet(RangeList::from(2.0..=3.0));
		assert_eq!(lit.to_string(), "2.0..3.0");
		let lit = Literal::<&str>::String(String::from("hello"));
		assert_eq!(lit.to_string(), "\"hello\"");

		let y = Arc::new(Array {
			name: "y".to_owned(),
			ann: vec![Annotation::Atom("special")],
			contents: vec![Literal::Int(1), Literal::Int(2), Literal::Int(3)],
			introduced: true,
			defined: true,
		});
		let fzn = FlatZinc {
			variables: vec![Arc::clone(&x)],
			arrays: vec![Arc::clone(&y)],
			output: vec![NamedRef::Array(Arc::clone(&y))],
			..Default::default()
		};
		assert_eq!(
			fzn.to_string(),
			"var set of int: x ::var_is_introduced ::special;\narray[1..3] of int: y ::output_array([1..3]) ::is_defined_var ::var_is_introduced ::special = [1, 2, 3];\nsolve satisfy;\n"
		);

		let sat = SolveObjective {
			method: Method::Minimize(Literal::Variable(x)),
			ann: vec![ann],
		};
		assert_eq!(
			sat.to_string(),
			"solve ::bool_search(input_order, indomain_min) minimize x"
		);
	}

	fn test_successful_serialization(file: &Path, exp: ExpectFile) {
		let rdr = BufReader::new(File::open(file).unwrap());
		let fzn: FlatZinc = serde_json::from_reader(rdr).unwrap();
		exp.assert_debug_eq(&fzn);
		let fzn2: FlatZinc = serde_json::from_str(&serde_json::to_string(&fzn).unwrap()).unwrap();
		assert_eq!(fzn, fzn2)
	}

	test_file!(documentation_example);
	test_file!(encapsulated_string);
	test_file!(float_sets);
	test_file!(set_literals);
	test_file!(unit_test_example);
}
