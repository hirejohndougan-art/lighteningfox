//! CSS custom properties (`--name: value`) an element has, shared with its ancestors.
//!
//! Every element inherits all of its parent's custom properties, and an element that declares
//! a few of its own used to copy the whole inherited map. The New York Times (2026-11) puts
//! about 900 on the root and some 10,000 elements declare one or two more, so each style pass
//! held gigabytes and the page ran out of memory. Here an element that declares any keeps only
//! its own (a layer) and points at the inherited ones, so lookups walk up a few layers.

use std::collections::HashMap;
use std::rc::Rc;

/// More layers than this are folded into one, so a lookup stays short on deep pages.
const MAX_DEPTH: usize = 24;

#[derive(Debug, Clone, Default)]
pub struct Vars {
    /// This layer's values; `None` unsets an inherited one (`--x: initial`).
    own: HashMap<String, Option<String>>,
    parent: Option<Rc<Vars>>,
    depth: usize,
}

impl Vars {
    /// An empty layer on top of `parent`.
    pub fn over(parent: Rc<Vars>) -> Vars {
        let depth = parent.depth + 1;
        Vars { own: HashMap::new(), parent: Some(parent), depth }
    }

    pub fn get(&self, name: &str) -> Option<&String> {
        let mut cur = self;
        loop {
            if let Some(v) = cur.own.get(name) {
                return v.as_ref();
            }
            cur = cur.parent.as_deref()?;
        }
    }

    pub fn insert(&mut self, name: String, value: String) {
        self.own.insert(name, Some(value));
    }

    pub fn remove(&mut self, name: &str) {
        if self.parent.as_ref().is_some_and(|p| p.get(name).is_some()) {
            self.own.insert(name.to_string(), None);
        } else {
            self.own.remove(name);
        }
    }

    /// Every property in effect (nearest layer wins), in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        let mut seen: HashMap<&String, Option<&String>> = HashMap::new();
        let mut cur = Some(self);
        while let Some(l) = cur {
            for (k, v) in &l.own {
                seen.entry(k).or_insert(v.as_ref());
            }
            cur = l.parent.as_deref();
        }
        seen.into_iter().filter_map(|(k, v)| v.map(|v| (k, v)))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// The same properties in a single layer.
    fn flattened(&self) -> Vars {
        let own = self.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect();
        Vars { own, parent: None, depth: 0 }
    }
}

/// A layer of `vars` this element can change without touching what it inherited: when the
/// map is shared (inherited as is), a new empty layer goes on top of it first.
pub fn own_layer(vars: &mut Rc<Vars>) -> &mut Vars {
    if Rc::strong_count(vars) > 1 || Rc::weak_count(vars) > 0 {
        let next = if vars.depth >= MAX_DEPTH { Vars::over(Rc::new(vars.flattened())) } else { Vars::over(vars.clone()) };
        *vars = Rc::new(next);
    }
    Rc::get_mut(vars).expect("unshared")
}

/// Something `var(--name)` can be looked up in.
pub trait VarSource {
    fn var(&self, name: &str) -> Option<&str>;
}

impl VarSource for Vars {
    fn var(&self, name: &str) -> Option<&str> {
        self.get(name).map(String::as_str)
    }
}

impl VarSource for HashMap<String, String> {
    fn var(&self, name: &str) -> Option<&str> {
        self.get(name).map(String::as_str)
    }
}

impl<T: VarSource> VarSource for Rc<T> {
    fn var(&self, name: &str) -> Option<&str> {
        (**self).var(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_share_and_shadow() {
        let mut root = Rc::new(Vars::default());
        own_layer(&mut root).insert("--a".into(), "1".into());
        own_layer(&mut root).insert("--b".into(), "2".into());
        let mut child = root.clone();
        own_layer(&mut child).insert("--a".into(), "9".into());
        own_layer(&mut child).remove("--b");
        assert_eq!(root.get("--a").map(String::as_str), Some("1"));
        assert_eq!(root.get("--b").map(String::as_str), Some("2"));
        assert_eq!(child.get("--a").map(String::as_str), Some("9"));
        assert_eq!(child.get("--b"), None);
        assert_eq!(child.len(), 1);
        // Deep chains fold into one layer but keep every value.
        let mut v = root.clone();
        for i in 0..100 {
            let keep = v.clone();
            own_layer(&mut v).insert(format!("--l{i}"), i.to_string());
            drop(keep);
        }
        assert!(v.depth <= MAX_DEPTH + 1);
        assert_eq!(v.get("--l0").map(String::as_str), Some("0"));
        assert_eq!(v.get("--a").map(String::as_str), Some("1"));
        assert_eq!(v.len(), 102);
    }
}
