//! Command history with coalescing, so a 300-event drag undoes in one step.

use std::fmt::Debug;

/// A reversible edit applied to a model `M`.
pub trait Command<M>: Debug {
    /// Applies the edit.
    fn apply(&self, model: &mut M);
    /// Reverts the edit.
    fn revert(&self, model: &mut M);
    /// Identifier used to merge consecutive edits of the same thing. `None` never merges.
    fn coalesce_key(&self) -> Option<String> {
        None
    }
    /// Merges a later edit with the same key into this one, keeping this edit's "before" state
    /// and the later edit's "after" state. Returns `false` to refuse.
    fn merge(&mut self, _later: &dyn Command<M>) -> bool {
        false
    }
    /// Exposes the concrete type for `merge` implementations.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Undo/redo stack.
#[derive(Debug)]
pub struct History<M> {
    undo: Vec<Box<dyn Command<M>>>,
    redo: Vec<Box<dyn Command<M>>>,
    limit: usize,
    /// When false, the next push never merges even if keys match (e.g. after a mouse-up).
    coalescing_open: bool,
}

impl<M> Default for History<M> {
    fn default() -> Self {
        Self::new(500)
    }
}

impl<M> History<M> {
    /// Creates a history keeping at most `limit` undo steps.
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: limit.max(1),
            coalescing_open: false,
        }
    }

    /// Applies `cmd` to `model` and records it.
    pub fn push(&mut self, model: &mut M, cmd: Box<dyn Command<M>>) {
        cmd.apply(model);
        self.redo.clear();
        if self.coalescing_open {
            if let (Some(last), Some(key)) = (self.undo.last_mut(), cmd.coalesce_key()) {
                if last.coalesce_key().as_deref() == Some(key.as_str()) && last.merge(cmd.as_ref())
                {
                    return;
                }
            }
        }
        self.undo.push(cmd);
        self.coalescing_open = true;
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Ends the current coalescing run; the next edit starts a fresh undo step.
    pub fn commit(&mut self) {
        self.coalescing_open = false;
    }

    /// Reverts the most recent step. Returns `false` when there is nothing to undo.
    pub fn undo(&mut self, model: &mut M) -> bool {
        self.coalescing_open = false;
        let Some(cmd) = self.undo.pop() else {
            return false;
        };
        cmd.revert(model);
        self.redo.push(cmd);
        true
    }

    /// Re-applies the most recently undone step.
    pub fn redo(&mut self, model: &mut M) -> bool {
        self.coalescing_open = false;
        let Some(cmd) = self.redo.pop() else {
            return false;
        };
        cmd.apply(model);
        self.undo.push(cmd);
        true
    }

    /// Number of undoable steps.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Number of redoable steps.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// True when there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// True when there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Drops all history.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.coalescing_open = false;
    }
}

/// A generic "set a value" command built from closures over a path key.
///
/// Most editor edits are of this form: remember the old value, write the new one.
#[derive(Debug, Clone)]
pub struct SetValue<T: Clone + Debug + Send + 'static> {
    /// Coalescing key (e.g. the parameter address).
    pub key: String,
    /// Value before the edit.
    pub before: T,
    /// Value after the edit.
    pub after: T,
}

/// Models that expose values by string key, so [`SetValue`] can be generic.
pub trait KeyedValues<T> {
    /// Writes the value for `key`.
    fn set_keyed(&mut self, key: &str, value: &T);
}

impl<M, T> Command<M> for SetValue<T>
where
    M: KeyedValues<T>,
    T: Clone + Debug + Send + 'static,
{
    fn apply(&self, model: &mut M) {
        model.set_keyed(&self.key, &self.after);
    }
    fn revert(&self, model: &mut M) {
        model.set_keyed(&self.key, &self.before);
    }
    fn coalesce_key(&self) -> Option<String> {
        Some(self.key.clone())
    }
    fn merge(&mut self, later: &dyn Command<M>) -> bool {
        match later.as_any().downcast_ref::<Self>() {
            Some(l) if l.key == self.key => {
                self.after = l.after.clone();
                true
            }
            _ => false,
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default, Debug)]
    struct Model(HashMap<String, f64>);

    impl KeyedValues<f64> for Model {
        fn set_keyed(&mut self, key: &str, value: &f64) {
            self.0.insert(key.to_string(), *value);
        }
    }

    fn set(key: &str, before: f64, after: f64) -> Box<dyn Command<Model>> {
        Box::new(SetValue {
            key: key.into(),
            before,
            after,
        })
    }

    #[test]
    fn apply_undo_redo() {
        let mut m = Model::default();
        let mut h = History::default();
        assert!(!h.can_undo() && !h.can_redo());
        h.push(&mut m, set("x", 0.0, 1.0));
        h.commit();
        h.push(&mut m, set("x", 1.0, 2.0));
        assert_eq!(m.0["x"], 2.0);
        assert_eq!(h.undo_len(), 2);
        assert!(h.undo(&mut m));
        assert_eq!(m.0["x"], 1.0);
        assert!(h.can_redo());
        assert!(h.redo(&mut m));
        assert_eq!(m.0["x"], 2.0);
        assert!(h.undo(&mut m));
        assert!(h.undo(&mut m));
        assert_eq!(m.0["x"], 0.0);
        assert!(!h.undo(&mut m));
        assert_eq!(h.redo_len(), 2);
        h.clear();
        assert!(!h.redo(&mut m));
    }

    #[test]
    fn drag_coalesces_into_one_step() {
        let mut m = Model::default();
        let mut h = History::default();
        for i in 0..300 {
            h.push(&mut m, set("x", f64::from(i), f64::from(i + 1)));
        }
        assert_eq!(h.undo_len(), 1);
        assert_eq!(m.0["x"], 300.0);
        assert!(h.undo(&mut m));
        assert_eq!(
            m.0["x"], 0.0,
            "undo goes back to the value before the drag started"
        );
        assert!(h.redo(&mut m));
        assert_eq!(m.0["x"], 300.0);
    }

    #[test]
    fn commit_and_different_keys_split_steps() {
        let mut m = Model::default();
        let mut h = History::default();
        h.push(&mut m, set("x", 0.0, 1.0));
        h.push(&mut m, set("y", 0.0, 1.0));
        h.push(&mut m, set("x", 1.0, 2.0));
        assert_eq!(h.undo_len(), 3);
        h.commit();
        h.push(&mut m, set("x", 2.0, 3.0));
        assert_eq!(h.undo_len(), 4);
        h.push(&mut m, set("x", 3.0, 4.0));
        assert_eq!(h.undo_len(), 4, "merges again after the commit boundary");
    }

    #[test]
    fn push_after_undo_discards_redo() {
        let mut m = Model::default();
        let mut h = History::default();
        h.push(&mut m, set("x", 0.0, 1.0));
        h.undo(&mut m);
        h.push(&mut m, set("x", 0.0, 5.0));
        assert!(!h.can_redo());
        assert_eq!(m.0["x"], 5.0);
    }

    #[test]
    fn limit_drops_oldest() {
        let mut m = Model::default();
        let mut h = History::new(2);
        for i in 0..5 {
            h.push(&mut m, set(&format!("k{i}"), 0.0, 1.0));
        }
        assert_eq!(h.undo_len(), 2);
        assert_eq!(History::<Model>::new(0).limit, 1);
    }

    #[derive(Debug)]
    struct Toggle;
    impl Command<Model> for Toggle {
        fn apply(&self, m: &mut Model) {
            *m.0.entry("t".into()).or_default() += 1.0;
        }
        fn revert(&self, m: &mut Model) {
            *m.0.entry("t".into()).or_default() -= 1.0;
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn default_trait_methods_never_merge() {
        let mut m = Model::default();
        let mut h = History::default();
        h.push(&mut m, Box::new(Toggle));
        h.push(&mut m, Box::new(Toggle));
        assert_eq!(h.undo_len(), 2);
        assert_eq!(m.0["t"], 2.0);
        // A SetValue refuses to merge with a foreign command of the same key.
        let mut sv = SetValue {
            key: "t".into(),
            before: 0.0,
            after: 1.0,
        };
        assert!(!Command::<Model>::merge(&mut sv, &Toggle));
        let other = SetValue {
            key: "u".into(),
            before: 0.0,
            after: 1.0,
        };
        assert!(!Command::<Model>::merge(&mut sv, &other));
    }
}
