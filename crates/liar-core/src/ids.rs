//! Typed indices into arenas.
//!
//! The engine stores nodes in `Vec`s and refers to them by index rather than by
//! reference. Indices are `Copy`, cannot dangle, do not borrow the arena, and
//! survive the arena growing — all of which matter for the cyclic structures an
//! AST and a control flow graph need.

use std::hash::Hash;
use std::marker::PhantomData;

/// A typed index into an [`Arena`].
pub trait Id: Copy + Eq + Ord + Hash {
    fn from_index(index: u32) -> Self;
    fn index(self) -> u32;
}

/// Declares a newtype index implementing [`Id`].
#[macro_export]
macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(u32);

        impl $crate::ids::Id for $name {
            fn from_index(index: u32) -> Self {
                Self(index)
            }

            fn index(self) -> u32 {
                self.0
            }
        }
    };
}

/// A growable, index-addressed store.
#[derive(Debug)]
pub struct Arena<I: Id, T> {
    items: Vec<T>,
    _marker: PhantomData<fn() -> I>,
}

impl<I: Id, T> Arena<I, T> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Appends `value` and returns its id.
    ///
    /// # Panics
    /// If the arena would exceed `u32::MAX` entries. A single Python file
    /// producing four billion nodes is a bug somewhere else.
    pub fn alloc(&mut self, value: T) -> I {
        let index = u32::try_from(self.items.len()).expect("arena exceeded u32::MAX entries");
        self.items.push(value);
        I::from_index(index)
    }

    /// # Panics
    /// If `id` did not come from this arena.
    pub fn get(&self, id: I) -> &T {
        self.items
            .get(id.index() as usize)
            .expect("id out of range for this arena")
    }

    /// # Panics
    /// If `id` did not come from this arena.
    pub fn get_mut(&mut self, id: I) -> &mut T {
        self.items
            .get_mut(id.index() as usize)
            .expect("id out of range for this arena")
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, item)| (I::from_index(i as u32), item))
    }
}

impl<I: Id, T> Default for Arena<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

define_id!(FileId);
define_id!(NodeId);

#[cfg(test)]
mod tests {
    use super::*;

    define_id!(TestId);

    #[test]
    fn alloc_returns_sequential_ids() {
        let mut arena: Arena<TestId, &str> = Arena::new();
        let a = arena.alloc("a");
        let b = arena.alloc("b");
        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);
        assert_ne!(a, b);
    }

    #[test]
    fn get_returns_the_allocated_value() {
        let mut arena: Arena<TestId, u32> = Arena::new();
        let id = arena.alloc(42);
        assert_eq!(*arena.get(id), 42);
    }

    #[test]
    fn ids_remain_valid_after_further_allocation() {
        // The whole point of indices over references: growing the arena
        // cannot invalidate an id handed out earlier.
        let mut arena: Arena<TestId, u32> = Arena::new();
        let first = arena.alloc(1);
        for n in 2..1000 {
            arena.alloc(n);
        }
        assert_eq!(*arena.get(first), 1);
    }

    #[test]
    fn get_mut_mutates_in_place() {
        let mut arena: Arena<TestId, u32> = Arena::new();
        let id = arena.alloc(1);
        *arena.get_mut(id) = 7;
        assert_eq!(*arena.get(id), 7);
    }

    #[test]
    fn iter_yields_every_item_with_its_id_in_order() {
        let mut arena: Arena<TestId, char> = Arena::new();
        let a = arena.alloc('a');
        let b = arena.alloc('b');
        let collected: Vec<_> = arena.iter().collect();
        assert_eq!(collected, vec![(a, &'a'), (b, &'b')]);
    }

    #[test]
    fn empty_arena_reports_empty() {
        let arena: Arena<TestId, u32> = Arena::new();
        assert!(arena.is_empty());
        assert_eq!(arena.len(), 0);
    }

    #[test]
    #[should_panic(expected = "id out of range")]
    fn get_with_a_foreign_id_panics_loudly() {
        // A bug, not a recoverable condition. Panicking with a clear message
        // beats returning an Option every caller unwraps.
        let arena: Arena<TestId, u32> = Arena::new();
        let _ = arena.get(TestId::from_index(0));
    }

    #[test]
    fn ids_sort_by_index() {
        let mut ids = vec![
            TestId::from_index(2),
            TestId::from_index(0),
            TestId::from_index(1),
        ];
        ids.sort();
        assert_eq!(
            ids,
            vec![
                TestId::from_index(0),
                TestId::from_index(1),
                TestId::from_index(2)
            ]
        );
    }
}
