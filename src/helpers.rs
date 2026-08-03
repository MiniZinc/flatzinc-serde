//! Helper structures, methods, and functions to support the serialization and
//! deserialization of FlatZinc data.

use std::{
	borrow::Cow,
	fmt::Debug,
	hash::{Hash, Hasher},
	ops::Deref,
	sync::{Arc, PoisonError, RwLock},
};

/// A family of smart pointers used to share the [`Variable`](crate::Variable)
/// and [`Array`](crate::Array) declarations of a
/// [`FlatZinc`](crate::FlatZinc) instance.
///
/// This is the trait behind the `Ref` type parameter of the public types. It
/// selects whether the shared declarations are plain [`Immutable`] `Arc<T>`
/// values, as produced by parsing, or [`Mutable`] `Arc<RwLock<T>>` values that
/// can be edited in place after the instance has been constructed.
///
/// The supertraits are free — implementors are zero-sized markers — and let the
/// derived `Clone`, `Debug`, and `PartialEq` implementations on the public
/// types, which bound every type parameter, be satisfied by `Ref: FznRef`
/// alone.
pub trait FznRef: Copy + Debug + Eq {
	/// The reference type used to share a `T`.
	type Of<T>: Clone;

	/// Allocate a new shared declaration.
	fn new<T>(value: T) -> Self::Of<T>;

	/// Borrow the shared declaration for reading, and apply `f` to it.
	fn with<T, R>(node: &Self::Of<T>, f: impl FnOnce(&T) -> R) -> R;

	/// The address of the shared allocation, used to test pointer identity.
	///
	/// Comparing two references with [`FznRef::with`] would take two read locks
	/// under [`Mutable`], which deadlocks if both name the same declaration and
	/// a writer is waiting. Testing this first avoids that.
	///
	/// This returns a bare address rather than a pointer, matching the
	/// `addr` method on raw pointers, because the result is only ever compared:
	/// keeping it a pointer would carry provenance that no caller is entitled
	/// to use.
	fn addr<T>(node: &Self::Of<T>) -> usize;

	/// Project a string field out of the shared declaration.
	///
	/// The result borrows from `node` for [`Immutable`], but must be cloned for
	/// [`Mutable`], where no borrow can escape the lock guard.
	///
	/// This exists solely for [`NamedRef::name`](crate::NamedRef::name), which
	/// must hand back a value that outlives the call. Anything that consumes
	/// the name in place should use [`FznRef::with`] instead, which allocates
	/// under neither marker.
	fn map_str<'a, T>(node: &'a Self::Of<T>, f: impl FnOnce(&T) -> &str) -> Cow<'a, str>;
}

/// Marker selecting immutable shared declarations, represented as `Arc<T>`.
///
/// This is the default, and the representation produced by parsing. Reading a
/// declaration is a plain pointer dereference.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Immutable;

/// Marker selecting interior-mutable shared declarations, represented as
/// `Arc<RwLock<T>>`.
///
/// This allows a [`Variable`](crate::Variable) or [`Array`](crate::Array) to be
/// edited after the instance has been constructed, with every reference to that
/// declaration observing the change. Since `Mutable::Of<T>` is simply
/// `Arc<RwLock<T>>`, edits are made through the normal lock API:
///
/// ```
/// # use std::sync::{Arc, RwLock};
/// # use flatzinc_serde::{Type, Variable, helpers::Mutable};
/// # let var: Arc<RwLock<Variable<String, Mutable>>> = Arc::new(RwLock::new(Variable {
/// #     name: "x".to_owned(),
/// #     ty: Type::Int(None),
/// #     ann: Vec::new(),
/// #     defined: false,
/// #     introduced: false,
/// # }));
/// var.write().unwrap().ty = Type::Bool;
/// ```
///
/// ### Warning
///
/// Reading a declaration takes a read lock, and several trait implementations
/// do so implicitly: [`Display`](std::fmt::Display) and `Serialize` on any type
/// that reaches a declaration, and [`Hash`], [`Ord`], and [`PartialEq`] on
/// [`NamedRef`](crate::NamedRef), which read the declaration name. Holding a
/// write guard on a declaration while invoking any of these on a value that
/// reaches the same declaration will deadlock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mutable;

impl FznRef for Immutable {
	type Of<T> = Arc<T>;

	fn new<T>(value: T) -> Self::Of<T> {
		Arc::new(value)
	}

	fn with<T, R>(node: &Self::Of<T>, f: impl FnOnce(&T) -> R) -> R {
		f(node)
	}

	fn addr<T>(node: &Self::Of<T>) -> usize {
		Arc::as_ptr(node).addr()
	}

	fn map_str<'a, T>(node: &'a Self::Of<T>, f: impl FnOnce(&T) -> &str) -> Cow<'a, str> {
		Cow::Borrowed(f(node))
	}
}

impl FznRef for Mutable {
	type Of<T> = Arc<RwLock<T>>;

	fn new<T>(value: T) -> Self::Of<T> {
		Arc::new(RwLock::new(value))
	}

	fn with<T, R>(node: &Self::Of<T>, f: impl FnOnce(&T) -> R) -> R {
		f(&node.read().unwrap_or_else(PoisonError::into_inner))
	}

	fn addr<T>(node: &Self::Of<T>) -> usize {
		Arc::as_ptr(node).addr()
	}

	fn map_str<'a, T>(node: &'a Self::Of<T>, f: impl FnOnce(&T) -> &str) -> Cow<'a, str> {
		Cow::Owned(f(&node.read().unwrap_or_else(PoisonError::into_inner)).to_owned())
	}
}

/// A wrapper around an [`Arc`] that can be used as a key for collections, such
/// as [`BTreeMap`](std::collections::BTreeMap),
/// [`HashMap`](std::collections::HashMap), and
/// [`HashSet`](std::collections::HashSet).
///
/// This struct uses pointer equality for comparison, so two [`ArcKey`]
/// instances from [`Arc`] objects that share the same value will be considered
/// equal. However, two `T` values with the same contents but different memory
/// addresses will not be considered equal.
#[derive(Debug, Clone)]
pub struct ArcKey<T> {
	/// The underlying [`Arc`] value.
	key: Arc<T>,
}

impl<T> From<ArcKey<T>> for Arc<T> {
	fn from(value: ArcKey<T>) -> Self {
		value.key
	}
}

impl<T> ArcKey<T> {
	/// Creates a new [`ArcKey`] from the given [`Arc`].
	pub fn new(key: Arc<T>) -> Self {
		Self { key }
	}
}

impl<T> Deref for ArcKey<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		self.key.deref()
	}
}

impl<T> Eq for ArcKey<T> {}

impl<T> Hash for ArcKey<T> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		Arc::as_ptr(&self.key).addr().hash(state);
	}
}

impl<T> Ord for ArcKey<T> {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		Arc::as_ptr(&self.key)
			.addr()
			.cmp(&Arc::as_ptr(&other.key).addr())
	}
}

impl<T> PartialEq for ArcKey<T> {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.key, &other.key)
	}
}

impl<T> PartialOrd for ArcKey<T> {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

#[cfg(test)]
mod tests {
	use std::{
		collections::{BTreeMap, HashMap, HashSet},
		sync::Arc,
	};

	use crate::helpers::ArcKey;

	#[test]
	fn arc_key() {
		let key = Arc::new(42);
		let arc_key = ArcKey::new(Arc::clone(&key));
		assert_eq!(*arc_key, 42);
		assert_eq!(arc_key.key, key);
	}

	#[test]
	fn arc_key_uses_pointer_equality() {
		let key = Arc::new(String::from("value"));
		let same_ptr = ArcKey::new(Arc::clone(&key));
		let also_same_ptr = ArcKey::new(Arc::clone(&key));
		let different_ptr = ArcKey::new(Arc::new(String::from("value")));

		assert_eq!(same_ptr, also_same_ptr);
		assert_ne!(same_ptr, different_ptr);
	}

	#[test]
	fn arc_key_works_in_btree_map() {
		let first = Arc::new(String::from("shared"));
		let second = Arc::new(String::from("shared"));
		let first_key = ArcKey::new(Arc::clone(&first));
		let second_key = ArcKey::new(Arc::clone(&second));

		let mut map = BTreeMap::new();
		let _ = map.insert(first_key.clone(), "first");
		let _ = map.insert(ArcKey::new(first), "updated");
		let _ = map.insert(second_key.clone(), "second");

		assert_eq!(map.len(), 2);
		assert_eq!(map.get(&first_key), Some(&"updated"));
		assert_eq!(map.get(&second_key), Some(&"second"));
	}

	#[test]
	fn arc_key_works_in_hash_map() {
		let key = Arc::new(String::from("entry"));
		let same_ptr = ArcKey::new(Arc::clone(&key));
		let different_ptr = ArcKey::new(Arc::new(String::from("entry")));
		let mut map = HashMap::new();

		let _ = map.insert(same_ptr.clone(), 1);
		let _ = map.insert(ArcKey::new(Arc::clone(&key)), 2);
		let _ = map.insert(different_ptr.clone(), 3);

		assert_eq!(map.len(), 2);
		assert_eq!(map.get(&same_ptr), Some(&2));
		assert_eq!(map.get(&ArcKey::new(key)), Some(&2));
		assert_eq!(map.get(&different_ptr), Some(&3));
	}

	#[test]
	fn arc_key_works_in_hash_set() {
		let key = Arc::new(7_u32);
		let same_ptr = ArcKey::new(Arc::clone(&key));
		let different_ptr = ArcKey::new(Arc::new(7_u32));

		let mut set = HashSet::new();
		let _ = set.insert(same_ptr.clone());
		let _ = set.insert(ArcKey::new(key));
		let _ = set.insert(different_ptr.clone());

		assert_eq!(set.len(), 2);
		assert!(set.contains(&same_ptr));
		assert!(set.contains(&different_ptr));
	}
}
