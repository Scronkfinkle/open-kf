//! Default property values of classes, with inheritance.
//!
//! An actor's effective value for a property is its own (if saved in the map),
//! otherwise the default of the nearest class in its class chain that sets it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::package_set::{LoadedPackage, ObjectHandle, PackageSet};
use crate::properties::{PropertyList, Value, add_class_property_names, find_class_defaults};

struct ClassInfo {
    handle: ObjectHandle,
    defaults: Option<PropertyList>,
    super_class: Option<ObjectHandle>,
}

pub struct ClassDefaults<'a> {
    set: &'a PackageSet,
    property_names: HashSet<String>,
    cache: RefCell<HashMap<String, Rc<ClassInfo>>>,
    /// Classes whose defaults could not be located, for logging.
    pub not_found: RefCell<Vec<String>>,
}

impl<'a> ClassDefaults<'a> {
    /// Loads every script package to collect the names of all declared
    /// properties (needed to recognise where defaults start).
    pub fn new(set: &'a PackageSet) -> Self {
        let mut property_names = HashSet::new();
        for name in set.script_package_names() {
            if let Some(lp) = set.load(&name) {
                add_class_property_names(&lp.pkg, &mut property_names);
            }
        }
        ClassDefaults {
            set,
            property_names,
            cache: RefCell::new(HashMap::new()),
            not_found: RefCell::new(Vec::new()),
        }
    }

    fn class_info(&self, handle: &ObjectHandle) -> Rc<ClassInfo> {
        let key = handle.path();
        if let Some(info) = self.cache.borrow().get(&key) {
            return info.clone();
        }
        let pkg = &handle.package.pkg;
        let defaults = find_class_defaults(pkg, handle.export, &self.property_names).map(|(l, _)| l);
        if defaults.is_none() {
            self.not_found.borrow_mut().push(key.clone());
        }
        let super_class = self.set.resolve(&handle.package, pkg.exports[handle.export].super_ref);
        let info = Rc::new(ClassInfo {
            handle: handle.clone(),
            defaults,
            super_class,
        });
        self.cache.borrow_mut().insert(key, info.clone());
        info
    }

    /// The class object of an actor export.
    pub fn class_of(&self, lp: &Rc<LoadedPackage>, export: usize) -> Option<ObjectHandle> {
        self.set.resolve(lp, lp.pkg.exports[export].class)
    }

    /// Default value of `prop` for `class`, searching up the class chain.
    /// Returns the value and the package its names/references belong to.
    pub fn get(&self, class: &ObjectHandle, prop: &str) -> Option<(Value, Rc<LoadedPackage>)> {
        let mut current = Some(self.class_info(class));
        for _ in 0..32 {
            let info = current?;
            if let Some(list) = &info.defaults
                && let Some(v) = list.get(&info.handle.package.pkg, prop)
            {
                return Some((v.clone(), info.handle.package.clone()));
            }
            current = info.super_class.as_ref().map(|s| self.class_info(s));
        }
        None
    }

    /// All elements of a fixed-size array default (e.g. `MeleeAnims`,
    /// `MeleeAnims[1]`, ...) from the nearest class in the chain that sets
    /// any of them, as names, ordered by index.
    pub fn get_array_names(&self, class: &ObjectHandle, prop: &str) -> Vec<String> {
        let mut current = Some(self.class_info(class));
        for _ in 0..32 {
            let Some(info) = current else { break };
            if let Some(list) = &info.defaults {
                let pkg = &info.handle.package.pkg;
                let mut items: Vec<(u32, String)> = list
                    .props
                    .iter()
                    .filter(|p| pkg.name(p.name).eq_ignore_ascii_case(prop))
                    .filter_map(|p| match &p.value {
                        Value::Name(n) => Some((p.array_index, pkg.name(*n).to_string())),
                        _ => None,
                    })
                    .collect();
                if !items.is_empty() {
                    items.sort_by_key(|(i, _)| *i);
                    return items.into_iter().map(|(_, n)| n).collect();
                }
            }
            current = info.super_class.as_ref().map(|s| self.class_info(s));
        }
        Vec::new()
    }

    /// All elements of a fixed-size array default holding object references
    /// (e.g. `Splats`, `Splats[1]`, ...), resolved, ordered by index, from
    /// the nearest class in the chain that sets any of them.
    pub fn get_array_objects(&self, class: &ObjectHandle, prop: &str) -> Vec<ObjectHandle> {
        let mut current = Some(self.class_info(class));
        for _ in 0..32 {
            let Some(info) = current else { break };
            if let Some(list) = &info.defaults {
                let pkg = &info.handle.package.pkg;
                let mut items: Vec<(u32, ObjectHandle)> = list
                    .props
                    .iter()
                    .filter(|p| pkg.name(p.name).eq_ignore_ascii_case(prop))
                    .filter_map(|p| match &p.value {
                        Value::Object(r) => self.set.resolve(&info.handle.package, *r).map(|h| (p.array_index, h)),
                        _ => None,
                    })
                    .collect();
                if !items.is_empty() {
                    items.sort_by_key(|(i, _)| *i);
                    return items.into_iter().map(|(_, h)| h).collect();
                }
            }
            current = info.super_class.as_ref().map(|s| self.class_info(s));
        }
        Vec::new()
    }

    /// Effective value for an actor: its own saved property, else the class default.
    pub fn actor_value(
        &self,
        lp: &Rc<LoadedPackage>,
        export: usize,
        own: &PropertyList,
        prop: &str,
    ) -> Option<Value> {
        if let Some(v) = own.get(&lp.pkg, prop) {
            return Some(v.clone());
        }
        let class = self.class_of(lp, export)?;
        self.get(&class, prop).map(|(v, _)| v)
    }

    /// Convenience: effective boolean, false if unset everywhere.
    pub fn actor_bool(&self, lp: &Rc<LoadedPackage>, export: usize, own: &PropertyList, prop: &str) -> bool {
        matches!(self.actor_value(lp, export, own, prop), Some(Value::Bool(true)))
    }
}
