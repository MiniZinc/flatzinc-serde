//! Helper structures, methods, and functions to support the serialization and
//! deserialization of FlatZinc data.

use std::{
	hash::{Hash, Hasher},
	ops::Deref,
	sync::Arc,
};

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
		Arc::as_ptr(&self.key).hash(state);
	}
}

impl<T> Ord for ArcKey<T> {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		Arc::as_ptr(&self.key).cmp(&Arc::as_ptr(&other.key))
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
