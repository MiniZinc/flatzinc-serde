# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## unreleased

## [0.5.0](https://github.com/MiniZinc/flatzinc-serde/compare/v0.4.4...v0.5.0) - 2026-04-17

### Added

- Add support for parsing `.fzn` files when enabling the `fzn` feature.
  Users can access this functionality via the `FlatZinc::from_fzn` method.
- Add helper type `ArcKey`, to use in collections that use variables or arrays as keys.
  `ArcKey` uses pointer identity to determine its order, equality, and hash value.
  Similarly, `NamedRef` can be used as a key, where the `name` attribute of variables and arrays are used to compare.

### Changed

- [**breaking**] The `domain` field of `Variable` has now moved to a variant argument on `Type`, accessible through the `ty` attribute.
- [**breaking**] The `objective` field of the `SolveMethod` struct has now moved to a variant argument on `Method`, accessible through the `method` attribute.
- [**breaking**] Change the default implementation of `variables` and `arrays` field of `FlatZinc` to be `std::collections::HashMap`.
- Allow the usage of stateful interners for `Identifier` using `FlatZinc::deserialize_with_interner` and `FlatZinc::from_fzn_with_interner`.
- [**breaking**] Remove the `value` field from `Variable`.
  MiniZinc 2.9.6 and later already resolve declaration right-hand sides before emitting FlatZinc, so this crate no longer accepts or exposes those values through `Variable`.
- [**breaking**] `FlatZinc` now uses `Arc<Variable>` and `Arc<Array>` to represent variable reference in `Literal`.
  This allowed the removal of the `Argument` type, as inline arrays are now represented as `Array` types without names.
	The `variables` and `arrays` attributes of `FlatZinc` are now `Vec<Arc<_>>`.
	`AnnotationLiteral` now contains all its own variants, it no longer has a direct `Literal` variant.
  This avoids strong references in annotations that might lead to self-referencing structures.

## [0.4.4] - 2025-11-06

### Changed

- Update `rangelist` dependency

## [0.4.3] - 2025-09-25

### Changed

- Update `rangelist` dependency

## [0.4.2] - 2025-08-13

### Changed

- Update `rangelist` dependency

## [0.4.1] - 2025-08-12

### Changed

- Update dependencies

## [0.4.0] - 2024-08-12

### Changed

- The `RangeList` type has been moved to a separate crate called `rangelist`.
  The `RangeList` type is now re-exported from the `flatzinc-serde` crate.

### Fixed

- Add the missing generic type for the `Annotation` variant of `AnnotationLiteral`.

## [0.3.0] - 2024-05-14

### Changed

- The type `Call` has been split into `AnnotationCall` and `Constraint` to allow the more recursive structure of annotation arguments.

### Added

- Add a `RangeList::iter` when its elements implement the `Copy` trait.
  This method provides an iterator over the intervals of the `RangeList` where the yielded elements are copied.

## [0.2.0] - 2024-04-11

### Changed

- The type for identifiers can now be chosen by the user using a generic type, to allow optimizations like interning.
  The chosen type must implement the `Serialize` and `Deserialize` traits to recursive keep these traits available.
  The default type for identifiers is `String`.

### Added

- All structures now implement `Display` and output the structures in the traditional FlatZinc format.

## [0.1.0] - 2024-01-19

### Added

- Add initial defintion of structs to represent FlatZinc JSON format, where `FlatZinc` is the root struct.

[unreleased]: https://github.com/shackle-rs/shackle/releases/compare/flatzinc-serde-v0.4.3......HEAD
[0.4.3]: https://github.com/shackle-rs/shackle/releases/compare/flatzinc-serde-v0.4.2...flatzinc-serde-v0.4.3
[0.4.2]: https://github.com/shackle-rs/shackle/releases/compare/flatzinc-serde-v0.4.1...flatzinc-serde-v0.4.2
[0.4.1]: https://github.com/shackle-rs/shackle/releases/compare/flatzinc-serde-v0.4.0...flatzinc-serde-v0.4.1
[0.4.0]: https://github.com/shackle-rs/shackle/releases/compare/flatzinc-serde-v0.3.0...flatzinc-serde-v0.4.0
[0.3.0]: https://github.com/shackle-rs/shackle/releases/compare/flatzinc-serde-v0.2.0...flatzinc-serde-v0.3.0
[0.2.0]: https://github.com/shackle-rs/shackle/releases/compare/flatzinc-serde-v0.1.0...flatzinc-serde-v0.2.0
[0.1.0]: https://github.com/shackle-rs/shackle/releases/tag/flatzinc-serde-v0.1.0
