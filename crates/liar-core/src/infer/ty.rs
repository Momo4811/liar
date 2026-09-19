//! The type lattice.
//!
//! Deliberately shallow. These checks ask "is this a boolean", "is this a
//! number", "is this a collection" — and nothing else — so the lattice answers
//! those and stops. A richer type system would be more work and would let the
//! checks reason about things they have no business concluding from.

use crate::span::Span;

/// A key identifying a class definition: its file and the span of its name.
pub type ClassKey = (crate::ids::FileId, Span);

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
    /// Top. Absorbing, and no check may fire on it.
    ///
    /// This is the silence rule made structural. It is not possible to forget
    /// it in one branch of one check, because the lattice carries it.
    Unknown,
    /// Bottom. No reachable value — a function that only raises, an empty join.
    Never,

    Int,
    Float,
    Complex,
    Str,
    Bytes,
    Bool,
    NoneType,
    Ellipsis,

    List,
    Dict,
    Set,
    Tuple,

    /// An instance of a class defined in this project.
    Instance(ClassKey),
}

impl Ty {
    /// The least upper bound of two types.
    ///
    /// Two different concrete types join to `Unknown` rather than to a union.
    /// There is nothing useful for these checks to conclude from `int | str`,
    /// and collapsing is both simpler and more conservative.
    pub fn join(&self, other: &Ty) -> Ty {
        if self == other {
            return self.clone();
        }
        match (self, other) {
            (Ty::Never, _) => other.clone(),
            (_, Ty::Never) => self.clone(),
            _ => Ty::Unknown,
        }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Ty::Unknown)
    }

    /// Whether a check is permitted to say anything about this type.
    pub fn is_known(&self) -> bool {
        !matches!(self, Ty::Unknown | Ty::Never)
    }

    pub fn is_bool(&self) -> bool {
        matches!(self, Ty::Bool)
    }

    /// Whether this is a number.
    ///
    /// `bool` is excluded even though Python's `bool` subclasses `int`. A name
    /// like `count` holding a boolean is exactly the confusion C3b exists to
    /// report, so treating it as numeric would silence the interesting case.
    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int | Ty::Float | Ty::Complex)
    }

    pub fn is_collection(&self) -> bool {
        matches!(self, Ty::List | Ty::Dict | Ty::Set | Ty::Tuple)
    }

    /// How the type is named in a diagnostic.
    pub fn name(&self) -> &'static str {
        match self {
            Ty::Unknown => "unknown",
            Ty::Never => "never",
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Complex => "complex",
            Ty::Str => "str",
            Ty::Bytes => "bytes",
            Ty::Bool => "bool",
            Ty::NoneType => "None",
            Ty::Ellipsis => "ellipsis",
            Ty::List => "list",
            Ty::Dict => "dict",
            Ty::Set => "set",
            Ty::Tuple => "tuple",
            Ty::Instance(_) => "an instance",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{FileId, Id};
    use proptest::prelude::*;

    fn every_type() -> Vec<Ty> {
        vec![
            Ty::Unknown,
            Ty::Never,
            Ty::Int,
            Ty::Float,
            Ty::Complex,
            Ty::Str,
            Ty::Bytes,
            Ty::Bool,
            Ty::NoneType,
            Ty::Ellipsis,
            Ty::List,
            Ty::Dict,
            Ty::Set,
            Ty::Tuple,
            Ty::Instance((FileId::from_index(0), Span::new(0, 1))),
            Ty::Instance((FileId::from_index(1), Span::new(4, 9))),
        ]
    }

    fn any_ty() -> impl Strategy<Value = Ty> {
        prop::sample::select(every_type())
    }

    // The five laws. Not stylistic: they are the preconditions for the fixpoint
    // converging, and for the answer not depending on the order files were
    // visited in.
    proptest! {
        #[test]
        fn join_is_commutative(a in any_ty(), b in any_ty()) {
            prop_assert_eq!(a.join(&b), b.join(&a));
        }

        #[test]
        fn join_is_associative(a in any_ty(), b in any_ty(), c in any_ty()) {
            prop_assert_eq!(a.join(&b).join(&c), a.join(&b.join(&c)));
        }

        #[test]
        fn join_is_idempotent(a in any_ty()) {
            prop_assert_eq!(a.join(&a), a);
        }

        #[test]
        fn never_is_the_identity(a in any_ty()) {
            prop_assert_eq!(Ty::Never.join(&a), a.clone());
            prop_assert_eq!(a.join(&Ty::Never), a);
        }

        #[test]
        fn unknown_is_absorbing(a in any_ty()) {
            prop_assert_eq!(Ty::Unknown.join(&a), Ty::Unknown);
            prop_assert_eq!(a.join(&Ty::Unknown), Ty::Unknown);
        }
    }

    #[test]
    fn two_different_concrete_types_collapse_to_unknown() {
        assert_eq!(Ty::Int.join(&Ty::Str), Ty::Unknown);
        assert_eq!(Ty::List.join(&Ty::Dict), Ty::Unknown);
    }

    #[test]
    fn distinct_instances_are_distinct_types() {
        let a = Ty::Instance((FileId::from_index(0), Span::new(0, 1)));
        let b = Ty::Instance((FileId::from_index(0), Span::new(9, 10)));
        assert_ne!(a, b);
        assert_eq!(a.join(&b), Ty::Unknown);
        assert_eq!(a.join(&a.clone()), a);
    }

    #[test]
    fn only_unknown_and_never_are_unspeakable() {
        for ty in every_type() {
            let speakable = ty.is_known();
            let expected = !matches!(ty, Ty::Unknown | Ty::Never);
            assert_eq!(speakable, expected, "for {ty:?}");
        }
    }

    #[test]
    fn a_bool_is_not_numeric() {
        // Python's bool subclasses int, but `count` holding a boolean is
        // exactly the confusion C3b exists to report. Treating it as numeric
        // would silence the interesting case.
        assert!(Ty::Bool.is_bool());
        assert!(!Ty::Bool.is_numeric());
        assert!(Ty::Int.is_numeric());
        assert!(!Ty::Int.is_bool());
    }

    #[test]
    fn collections_are_the_four_displays() {
        for ty in [Ty::List, Ty::Dict, Ty::Set, Ty::Tuple] {
            assert!(ty.is_collection(), "{ty:?}");
        }
        for ty in [Ty::Int, Ty::Str, Ty::Bool, Ty::NoneType, Ty::Unknown] {
            assert!(!ty.is_collection(), "{ty:?}");
        }
    }

    #[test]
    fn a_string_is_not_a_collection() {
        // It is iterable, but a name like `users` holding a str is a mistake
        // worth reporting, not a collection.
        assert!(!Ty::Str.is_collection());
    }

    #[test]
    fn every_type_has_a_name_for_a_message() {
        for ty in every_type() {
            assert!(!ty.name().is_empty(), "{ty:?}");
        }
    }
}
