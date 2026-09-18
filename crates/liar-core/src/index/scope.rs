//! The scope tree of one file.

use crate::define_id;
use crate::ids::Arena;
use crate::index::binding::Binding;
use crate::span::Span;
use std::collections::HashMap;

define_id!(ScopeId);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeKind {
    Module,
    Class,
    Function,
    /// A comprehension has its own scope in Python 3, which is why the loop
    /// variable of `[x for x in xs]` does not leak.
    Comprehension,
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    pub bindings: HashMap<String, Binding>,
    pub span: Span,
}

#[derive(Debug)]
pub struct ScopeTree {
    scopes: Arena<ScopeId, Scope>,
    root: ScopeId,
}

impl ScopeTree {
    /// Creates a tree with a module scope covering `span`.
    pub fn new(span: Span) -> Self {
        let mut scopes = Arena::new();
        let root = scopes.alloc(Scope {
            kind: ScopeKind::Module,
            parent: None,
            bindings: HashMap::new(),
            span,
        });
        Self { scopes, root }
    }

    pub fn root(&self) -> ScopeId {
        self.root
    }

    pub fn push(&mut self, kind: ScopeKind, parent: ScopeId, span: Span) -> ScopeId {
        self.scopes.alloc(Scope {
            kind,
            parent: Some(parent),
            bindings: HashMap::new(),
            span,
        })
    }

    /// Binds `name` in `scope`, replacing any previous binding.
    ///
    /// Rebinding replaces rather than accumulating: Python has one binding per
    /// name per scope, and a checker asking "what is this name" wants the
    /// answer, not a history.
    pub fn bind(&mut self, scope: ScopeId, name: impl Into<String>, binding: Binding) {
        self.scopes
            .get_mut(scope)
            .bindings
            .insert(name.into(), binding);
    }

    /// Looks in `scope` alone. Does not walk the parent chain — resolution
    /// does that, and it has rules this does not know about.
    pub fn lookup_local(&self, scope: ScopeId, name: &str) -> Option<&Binding> {
        self.scopes.get(scope).bindings.get(name)
    }

    pub fn scope(&self, id: ScopeId) -> &Scope {
        self.scopes.get(id)
    }

    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    pub fn is_empty(&self) -> bool {
        false // there is always a module scope
    }

    pub fn iter(&self) -> impl Iterator<Item = (ScopeId, &Scope)> {
        self.scopes.iter()
    }

    /// The scopes from `scope` up to the module, in lookup order.
    pub fn ancestry(&self, scope: ScopeId) -> Vec<ScopeId> {
        let mut chain = vec![scope];
        let mut current = scope;
        while let Some(parent) = self.scopes.get(current).parent {
            chain.push(parent);
            current = parent;
        }
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::binding::BindingKind;

    fn span() -> Span {
        Span::new(0, 10)
    }

    fn variable() -> Binding {
        Binding::new(BindingKind::Variable, span())
    }

    #[test]
    fn a_new_tree_has_a_module_scope_as_its_root() {
        let tree = ScopeTree::new(span());
        assert_eq!(tree.scope(tree.root()).kind, ScopeKind::Module);
        assert_eq!(tree.scope(tree.root()).parent, None);
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn a_function_introduces_a_scope_under_its_parent() {
        let mut tree = ScopeTree::new(span());
        let root = tree.root();
        let function = tree.push(ScopeKind::Function, root, span());
        assert_eq!(tree.scope(function).kind, ScopeKind::Function);
        assert_eq!(tree.scope(function).parent, Some(root));
    }

    #[test]
    fn a_class_introduces_a_scope() {
        let mut tree = ScopeTree::new(span());
        let root = tree.root();
        let class = tree.push(ScopeKind::Class, root, span());
        assert_eq!(tree.scope(class).kind, ScopeKind::Class);
    }

    #[test]
    fn nested_functions_nest() {
        let mut tree = ScopeTree::new(span());
        let root = tree.root();
        let outer = tree.push(ScopeKind::Function, root, span());
        let inner = tree.push(ScopeKind::Function, outer, span());
        assert_eq!(tree.scope(inner).parent, Some(outer));
        assert_eq!(tree.scope(outer).parent, Some(root));
    }

    #[test]
    fn a_binding_lands_in_the_scope_it_was_bound_in() {
        let mut tree = ScopeTree::new(span());
        let root = tree.root();
        let function = tree.push(ScopeKind::Function, root, span());
        tree.bind(function, "x", variable());

        assert!(tree.lookup_local(function, "x").is_some());
        assert!(
            tree.lookup_local(root, "x").is_none(),
            "must not leak upward"
        );
    }

    #[test]
    fn rebinding_replaces_rather_than_duplicating() {
        let mut tree = ScopeTree::new(span());
        let root = tree.root();
        tree.bind(
            root,
            "f",
            Binding::new(BindingKind::Function { is_async: false }, span()),
        );
        tree.bind(root, "f", Binding::new(BindingKind::Variable, span()));

        assert_eq!(
            tree.lookup_local(root, "f").unwrap().kind,
            BindingKind::Variable
        );
        assert_eq!(tree.scope(root).bindings.len(), 1);
    }

    #[test]
    fn lookup_local_does_not_walk_the_parent_chain() {
        // Resolution walks the chain, and it has rules about class scopes that
        // this method must not second-guess.
        let mut tree = ScopeTree::new(span());
        let root = tree.root();
        tree.bind(root, "g", variable());
        let function = tree.push(ScopeKind::Function, root, span());

        assert!(tree.lookup_local(function, "g").is_none());
        assert!(tree.lookup_local(root, "g").is_some());
    }

    #[test]
    fn ancestry_runs_from_the_scope_to_the_module() {
        let mut tree = ScopeTree::new(span());
        let root = tree.root();
        let class = tree.push(ScopeKind::Class, root, span());
        let method = tree.push(ScopeKind::Function, class, span());

        assert_eq!(tree.ancestry(method), vec![method, class, root]);
        assert_eq!(tree.ancestry(root), vec![root]);
    }

    #[test]
    fn unknown_names_are_absent_rather_than_invented() {
        let tree = ScopeTree::new(span());
        assert!(tree.lookup_local(tree.root(), "nope").is_none());
    }
}
