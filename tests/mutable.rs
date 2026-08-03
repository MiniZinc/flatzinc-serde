//! Tests for the opt-in [`Mutable`] reference family, which lets the shared
//! declarations of a parsed instance be edited in place.
//!
//! These exercise the public API only, which is the point: `Mutable` is an
//! opt-in a downstream crate selects by naming it in the type.

#![cfg(all(feature = "fzn", feature = "serde"))]
#![allow(
	clippy::tests_outside_test_module,
	reason = "these deliberately live outside `src`, so that they exercise `Mutable` the way a \
	          downstream crate would and catch anything missing from the public exports"
)]

use std::{
	fs::File,
	io::BufReader,
	path::Path,
	sync::{Arc, RwLock},
};

use flatzinc_serde::{
	Argument, FlatZinc, Literal, RangeList, Type,
	helpers::{ArcKey, Mutable},
};

/// A shared, interior-mutable variable declaration.
type MutVariable = Arc<RwLock<flatzinc_serde::Variable<String, Mutable>>>;

/// Parse the shared corpus example with the given reference family.
fn parse<Ref: flatzinc_serde::helpers::FznRef>() -> FlatZinc<String, Ref> {
	let path = Path::new("./corpus/fzn/documentation_example.fzn");
	FlatZinc::from_fzn(BufReader::new(File::open(path).unwrap())).unwrap()
}

/// Find the declaration of `name` in the instance's variable list.
fn variable(fzn: &FlatZinc<String, Mutable>, name: &str) -> MutVariable {
	fzn.variables
		.iter()
		.find(|var| var.read().unwrap().name == name)
		.map(Arc::clone)
		.unwrap_or_else(|| panic!("expected a variable named `{name}`"))
}

/// Find the variable `name` by walking into a constraint argument, rather than
/// through [`FlatZinc::variables`].
fn variable_via_constraint(fzn: &FlatZinc<String, Mutable>, name: &str) -> MutVariable {
	fzn.constraints
		.iter()
		.flat_map(|c| &c.args)
		.filter_map(|arg| match arg {
			Argument::Array(lits) => Some(lits),
			_ => None,
		})
		.flatten()
		.find_map(|lit| match lit {
			Literal::Variable(var) if var.read().unwrap().name == name => Some(Arc::clone(var)),
			_ => None,
		})
		.unwrap_or_else(|| panic!("expected `{name}` to appear in a constraint argument"))
}

#[test]
fn mutation_is_observed_through_every_reference() {
	let fzn: FlatZinc<String, Mutable> = parse();

	// `b` is declared as `var 0..3` and used inside the `int_lin_le` arrays.
	let narrowed = Type::Int(Some(RangeList::from(0..=1)));
	variable(&fzn, "b").write().unwrap().ty = narrowed.clone();

	// The same declaration, reached without going through `fzn.variables`.
	let via_constraint = variable_via_constraint(&fzn, "b");
	assert_eq!(via_constraint.read().unwrap().ty, narrowed);

	// And the edit is visible in the rendered model.
	assert!(fzn.to_string().contains("var 0..1: b"));
}

#[test]
fn pointer_identity_holds_under_mutable() {
	let fzn: FlatZinc<String, Mutable> = parse();

	assert!(Arc::ptr_eq(
		&variable(&fzn, "b"),
		&variable_via_constraint(&fzn, "b"),
	));
}

#[test]
fn output_matches_immutable_before_mutation() {
	let immutable: FlatZinc<String> = parse();
	let mutable: FlatZinc<String, Mutable> = parse();

	assert_eq!(immutable.to_string(), mutable.to_string());
	assert_eq!(
		serde_json::to_string(&immutable).unwrap(),
		serde_json::to_string(&mutable).unwrap(),
	);
}

#[test]
fn arc_key_keys_by_identity_under_mutable() {
	let fzn: FlatZinc<String, Mutable> = parse();

	let b = ArcKey::new(variable(&fzn, "b"));
	let same = ArcKey::new(variable_via_constraint(&fzn, "b"));
	let other = ArcKey::new(variable(&fzn, "c"));

	assert_eq!(b, same);
	assert_ne!(b, other);

	// `Deref` reaches the lock, so the declaration is still readable and
	// writable through the key itself.
	assert_eq!(b.read().unwrap().name, "b");
	b.write().unwrap().introduced = true;
	assert!(variable(&fzn, "b").read().unwrap().introduced);
}
