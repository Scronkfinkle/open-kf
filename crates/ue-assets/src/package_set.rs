//! Loads packages on demand and follows references between them.
//!
//! A map imports most of its textures and meshes from other packages, e.g.
//! `KillingFloorTextures.Vehicles.LondonAmbulanceDiffuseDDS`. The first part
//! of the path is the package name; the rest is the object's path inside it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::package::{ObjectRef, Package, PackageError};

/// Folders and extensions searched for packages, in priority order.
const SEARCH: &[(&str, &str)] = &[
    ("Textures", "utx"),
    ("StaticMeshes", "usx"),
    ("Animations", "ukx"),
    ("Sounds", "uax"),
    ("System", "u"),
    ("Maps", "rom"),
];

/// A package plus a lookup table from lowercase object path to export indices.
/// One path can name several objects of different classes (seen in
/// `Icebreaker_T.utx`: a Texture and a StaticMesh both called `ic_porte_02`).
pub struct LoadedPackage {
    pub name: String,
    pub pkg: Package,
    by_path: HashMap<String, Vec<usize>>,
}

impl LoadedPackage {
    fn new(name: String, pkg: Package) -> Self {
        let mut by_path: HashMap<String, Vec<usize>> = HashMap::new();
        for i in 0..pkg.exports.len() {
            by_path
                .entry(pkg.object_path(ObjectRef::Export(i)).to_ascii_lowercase())
                .or_default()
                .push(i);
        }
        LoadedPackage { name, pkg, by_path }
    }

    /// Export index for a dotted path inside this package, ignoring case.
    /// If `class` is given, only an object of that class matches.
    pub fn find(&self, path: &str, class: Option<&str>) -> Option<usize> {
        let candidates = self.by_path.get(&path.to_ascii_lowercase())?;
        candidates
            .iter()
            .copied()
            .find(|&i| class.is_none_or(|c| self.pkg.export_class_name(i).eq_ignore_ascii_case(c)))
    }
}

/// An object located in a specific package.
#[derive(Clone)]
pub struct ObjectHandle {
    pub package: Rc<LoadedPackage>,
    pub export: usize,
}

impl ObjectHandle {
    pub fn class_name(&self) -> &str {
        self.package.pkg.export_class_name(self.export)
    }

    pub fn path(&self) -> String {
        format!(
            "{}.{}",
            self.package.name,
            self.package.pkg.object_path(ObjectRef::Export(self.export))
        )
    }
}

pub struct PackageSet {
    files: HashMap<String, PathBuf>,
    loaded: RefCell<HashMap<String, Option<Rc<LoadedPackage>>>>,
    /// Packages that were referenced but could not be found or parsed.
    pub missing: RefCell<Vec<String>>,
}

impl PackageSet {
    /// Indexes every package file in the install (does not load them yet).
    pub fn new(root: &Path) -> Self {
        let mut files = HashMap::new();
        for &(dir, ext) in SEARCH {
            let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext))
                    && let Some(stem) = path.file_stem()
                {
                    // Earlier folders in SEARCH win if two share a name.
                    files.entry(stem.to_string_lossy().to_ascii_lowercase()).or_insert(path);
                }
            }
        }
        PackageSet {
            files,
            loaded: RefCell::new(HashMap::new()),
            missing: RefCell::new(Vec::new()),
        }
    }

    /// Loads a package from an explicit path (used for the map itself).
    pub fn load_path(&self, path: &Path) -> Result<Rc<LoadedPackage>, PackageError> {
        let name = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        let lp = Rc::new(LoadedPackage::new(name.clone(), Package::open(path)?));
        self.loaded
            .borrow_mut()
            .insert(name.to_ascii_lowercase(), Some(lp.clone()));
        Ok(lp)
    }

    /// Loads a package by name, e.g. `KillingFloorTextures`. Cached.
    pub fn load(&self, name: &str) -> Option<Rc<LoadedPackage>> {
        let key = name.to_ascii_lowercase();
        if let Some(cached) = self.loaded.borrow().get(&key) {
            return cached.clone();
        }
        let result = self
            .files
            .get(&key)
            .and_then(|path| Package::open(path).ok())
            .map(|pkg| Rc::new(LoadedPackage::new(name.to_string(), pkg)));
        if result.is_none() {
            self.missing.borrow_mut().push(name.to_string());
        }
        self.loaded.borrow_mut().insert(key, result.clone());
        result
    }

    /// Follows a reference found in `from` to the export that defines it.
    pub fn resolve(&self, from: &Rc<LoadedPackage>, rf: ObjectRef) -> Option<ObjectHandle> {
        match rf {
            ObjectRef::Null => None,
            ObjectRef::Export(i) => Some(ObjectHandle {
                package: from.clone(),
                export: i,
            }),
            ObjectRef::Import(i) => {
                let full = from.pkg.object_path(rf);
                let (pkg_name, inner) = full.split_once('.')?;
                let class = from.pkg.name(from.pkg.imports[i].class_name);
                let target = self.load(pkg_name)?;
                let export = target.find(inner, Some(class))?;
                Some(ObjectHandle { package: target, export })
            }
        }
    }

    /// Names of all script packages (`System/*.u`).
    pub fn script_package_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .files
            .iter()
            .filter(|(_, p)| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("u")))
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded.borrow().values().filter(|v| v.is_some()).count()
    }
}
