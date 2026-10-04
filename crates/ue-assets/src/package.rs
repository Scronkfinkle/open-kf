//! Unreal Engine 2.5 package files (`.u`, `.rom`, `.utx`, `.usx`, `.ukx`, `.uax`).
//!
//! A package has a header followed by three tables:
//! - **names**: every string the package uses (object names, class names, property names)
//! - **imports**: objects this package borrows from other packages
//! - **exports**: objects this package defines, each with the byte range of its data
//!
//! This module reads those tables and checks that they are consistent. Reading the
//! objects' contents is done elsewhere.

use std::fmt;
use std::path::Path;

use crate::reader::{ReadError, ReadErrorKind, Reader};

/// The standard Unreal package magic number.
pub const MAGIC_STANDARD: u32 = 0x9E2A_83C1;
/// The magic number used by almost every Killing Floor package (547 of 550 in
/// the tested install). The rest of the header is the standard layout.
pub const MAGIC_KILLING_FLOOR: u32 = 0x9E2A_83C2;

#[derive(Debug)]
pub enum PackageError {
    Io(std::io::Error),
    Read(ReadError),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::Io(e) => write!(f, "I/O error: {e}"),
            PackageError::Read(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<ReadError> for PackageError {
    fn from(e: ReadError) -> Self {
        PackageError::Read(e)
    }
}

impl From<std::io::Error> for PackageError {
    fn from(e: std::io::Error) -> Self {
        PackageError::Io(e)
    }
}

/// A reference to an object, as stored in packages.
/// Stored as an integer: 0 = none, positive = export index + 1, negative = -(import index + 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectRef {
    Null,
    Export(usize),
    Import(usize),
}

impl ObjectRef {
    pub fn from_raw(raw: i32) -> ObjectRef {
        match raw {
            0 => ObjectRef::Null,
            n if n > 0 => ObjectRef::Export((n - 1) as usize),
            n => ObjectRef::Import((-(n as i64) - 1) as usize),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Generation {
    pub export_count: i32,
    pub name_count: i32,
}

#[derive(Debug, Clone)]
pub struct NameEntry {
    pub name: String,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct Import {
    /// Name index of the package that defines the class, e.g. `Engine`.
    pub class_package: usize,
    /// Name index of the class, e.g. `Texture`.
    pub class_name: usize,
    /// The object containing this one (a package or group).
    pub outer: ObjectRef,
    pub object_name: usize,
}

#[derive(Debug, Clone)]
pub struct Export {
    /// The object's class. `Null` means the object is itself a class.
    pub class: ObjectRef,
    /// Parent class or struct, for classes, structs and functions.
    pub super_ref: ObjectRef,
    pub outer: ObjectRef,
    pub object_name: usize,
    pub object_flags: u32,
    /// Where the object's data lives in the file.
    pub serial_offset: usize,
    pub serial_size: usize,
}

/// A parsed package. Keeps the whole file in memory so object data can be
/// read later without reopening it.
pub struct Package {
    pub magic: u32,
    pub version: u16,
    pub licensee: u16,
    pub flags: u32,
    pub guid: [u8; 16],
    pub generations: Vec<Generation>,
    pub names: Vec<NameEntry>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    data: Vec<u8>,
}

impl fmt::Debug for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Package")
            .field("version", &self.version)
            .field("licensee", &self.licensee)
            .field("names", &self.names.len())
            .field("imports", &self.imports.len())
            .field("exports", &self.exports.len())
            .finish()
    }
}

/// Table counts above this are treated as corruption rather than allocated.
const MAX_TABLE_COUNT: i32 = 10_000_000;

impl Package {
    pub fn open(path: &Path) -> Result<Package, PackageError> {
        Package::parse(std::fs::read(path)?)
    }

    pub fn parse(data: Vec<u8>) -> Result<Package, PackageError> {
        let mut r = Reader::new(&data);

        let magic = r.u32()?;
        if magic != MAGIC_STANDARD && magic != MAGIC_KILLING_FLOOR {
            return Err(r.error(ReadErrorKind::Invalid(format!("not a package (magic {magic:#010x})"))).into());
        }
        let version = r.u16()?;
        let licensee = r.u16()?;
        if version < 68 {
            // Older layouts store the header differently. Nothing in KF uses them.
            return Err(r.error(ReadErrorKind::Invalid(format!("unsupported package version {version}"))).into());
        }
        let flags = r.u32()?;
        let name_count = table_count(&mut r, "name")?;
        let name_offset = table_offset(&mut r)?;
        let export_count = table_count(&mut r, "export")?;
        let export_offset = table_offset(&mut r)?;
        let import_count = table_count(&mut r, "import")?;
        let import_offset = table_offset(&mut r)?;
        let guid: [u8; 16] = r.bytes(16)?.try_into().expect("16 bytes");
        let generation_count = table_count(&mut r, "generation")?;
        let mut generations = Vec::with_capacity(generation_count);
        for _ in 0..generation_count {
            generations.push(Generation {
                export_count: r.i32()?,
                name_count: r.i32()?,
            });
        }

        r.seek(name_offset)?;
        let mut names = Vec::with_capacity(name_count);
        for _ in 0..name_count {
            names.push(NameEntry {
                name: r.fstring()?,
                flags: r.u32()?,
            });
        }

        r.seek(import_offset)?;
        let mut imports = Vec::with_capacity(import_count);
        for _ in 0..import_count {
            imports.push(Import {
                class_package: name_index(&mut r, name_count)?,
                class_name: name_index(&mut r, name_count)?,
                outer: ObjectRef::from_raw(r.i32()?),
                object_name: name_index(&mut r, name_count)?,
            });
        }

        r.seek(export_offset)?;
        let mut exports = Vec::with_capacity(export_count);
        for _ in 0..export_count {
            let class = ObjectRef::from_raw(r.compact_index()?);
            let super_ref = ObjectRef::from_raw(r.compact_index()?);
            let outer = ObjectRef::from_raw(r.i32()?);
            let object_name = name_index(&mut r, name_count)?;
            let object_flags = r.u32()?;
            let size_at = r.pos();
            let serial_size = r.compact_index()?;
            if serial_size < 0 {
                return Err(ReadError {
                    offset: size_at,
                    kind: ReadErrorKind::Invalid(format!("negative export size {serial_size}")),
                }
                .into());
            }
            let serial_offset = if serial_size > 0 { r.compact_index()? } else { 0 };
            let (serial_size, serial_offset) = (serial_size as usize, serial_offset as usize);
            if serial_size > 0 && (serial_offset.checked_add(serial_size).is_none_or(|end| end > data.len())) {
                return Err(ReadError {
                    offset: size_at,
                    kind: ReadErrorKind::Invalid(format!(
                        "export data {serial_offset}+{serial_size} runs past end of file ({} bytes)",
                        data.len()
                    )),
                }
                .into());
            }
            exports.push(Export {
                class,
                super_ref,
                outer,
                object_name,
                object_flags,
                serial_offset,
                serial_size,
            });
        }

        let package = Package {
            magic,
            version,
            licensee,
            flags,
            guid,
            generations,
            names,
            imports,
            exports,
            data,
        };
        package.check_refs()?;
        Ok(package)
    }

    /// Makes sure every object reference points at a real table entry.
    fn check_refs(&self) -> Result<(), PackageError> {
        let check = |what: &str, i: usize, rf: ObjectRef| -> Result<(), PackageError> {
            let ok = match rf {
                ObjectRef::Null => true,
                ObjectRef::Export(n) => n < self.exports.len(),
                ObjectRef::Import(n) => n < self.imports.len(),
            };
            if ok {
                Ok(())
            } else {
                Err(PackageError::Read(ReadError {
                    offset: 0,
                    kind: ReadErrorKind::Invalid(format!("{what} {i} has out-of-range reference {rf:?}")),
                }))
            }
        };
        for (i, imp) in self.imports.iter().enumerate() {
            check("import", i, imp.outer)?;
        }
        for (i, exp) in self.exports.iter().enumerate() {
            check("export", i, exp.class)?;
            check("export", i, exp.super_ref)?;
            check("export", i, exp.outer)?;
        }
        Ok(())
    }

    /// The raw file bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The bytes of one export's serialized data.
    pub fn export_data(&self, index: usize) -> &[u8] {
        let e = &self.exports[index];
        &self.data[e.serial_offset..e.serial_offset + e.serial_size]
    }

    pub fn name(&self, index: usize) -> &str {
        &self.names[index].name
    }

    /// The object's own name, e.g. `KF-Farm` or `ClotSkin`.
    pub fn object_name(&self, rf: ObjectRef) -> &str {
        match rf {
            ObjectRef::Null => "None",
            ObjectRef::Export(i) => self.name(self.exports[i].object_name),
            ObjectRef::Import(i) => self.name(self.imports[i].object_name),
        }
    }

    /// The full dotted path, e.g. `KFCharacters.Clot.ClotSkin`.
    pub fn object_path(&self, rf: ObjectRef) -> String {
        let mut parts = Vec::new();
        let mut cur = rf;
        // Guard against reference loops in a corrupt file.
        for _ in 0..64 {
            if cur == ObjectRef::Null {
                break;
            }
            parts.push(self.object_name(cur));
            cur = match cur {
                ObjectRef::Export(i) => self.exports[i].outer,
                ObjectRef::Import(i) => self.imports[i].outer,
                ObjectRef::Null => unreachable!(),
            };
        }
        parts.reverse();
        parts.join(".")
    }

    /// The class name of an export, e.g. `Texture`. Classes themselves report `Class`.
    pub fn export_class_name(&self, index: usize) -> &str {
        match self.exports[index].class {
            ObjectRef::Null => "Class",
            rf => self.object_name(rf),
        }
    }
}

fn table_count(r: &mut Reader, what: &str) -> Result<usize, ReadError> {
    let at = r.pos();
    let n = r.i32()?;
    if !(0..=MAX_TABLE_COUNT).contains(&n) {
        return Err(ReadError {
            offset: at,
            kind: ReadErrorKind::Invalid(format!("bad {what} count {n}")),
        });
    }
    Ok(n as usize)
}

fn table_offset(r: &mut Reader) -> Result<usize, ReadError> {
    let at = r.pos();
    let n = r.i32()?;
    if n < 0 || n as usize > r.len() {
        return Err(ReadError {
            offset: at,
            kind: ReadErrorKind::Invalid(format!("table offset {n} outside file")),
        });
    }
    Ok(n as usize)
}

fn name_index(r: &mut Reader, name_count: usize) -> Result<usize, ReadError> {
    let at = r.pos();
    let n = r.compact_index()?;
    if n < 0 || n as usize >= name_count {
        return Err(ReadError {
            offset: at,
            kind: ReadErrorKind::Invalid(format!("name index {n} out of range (0..{name_count})")),
        });
    }
    Ok(n as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal valid package: names [None, Core, Class, Thing],
    /// one import (Core.Class), one export (Thing, 3 bytes of data).
    fn tiny_package() -> Vec<u8> {
        let mut names = Vec::new();
        for n in ["None", "Core", "Class", "Thing"] {
            names.push(n.len() as u8 + 1);
            names.extend_from_slice(n.as_bytes());
            names.push(0);
            names.extend_from_slice(&0u32.to_le_bytes());
        }
        let header_len = 4 + 2 + 2 + 4 + 6 * 4 + 16 + 4 + 8;
        let name_off = header_len;
        let import_off = name_off + names.len();
        // import: class_package=1 (Core), class_name=2 (Class), outer=0, object_name=2
        let imports = [1u8, 2, 0, 0, 0, 0, 2];
        let export_off = import_off + imports.len();
        let data_off = export_off + 14;
        // export: class=-1 (import 0), super=0, outer=0 (i32), name=3, flags (u32), size=3, offset
        let mut exports = vec![0x81u8, 0];
        exports.extend_from_slice(&0i32.to_le_bytes());
        exports.push(3);
        exports.extend_from_slice(&0u32.to_le_bytes());
        exports.push(3);
        // offset is a compact index; values >= 64 need two bytes
        assert!((64..8192).contains(&data_off));
        exports.push((data_off & 0x3f) as u8 | 0x40);
        exports.push((data_off >> 6) as u8);
        assert_eq!(exports.len(), 14);

        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC_KILLING_FLOOR.to_le_bytes());
        out.extend_from_slice(&128u16.to_le_bytes());
        out.extend_from_slice(&29u16.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for v in [4, name_off, 1, export_off, 1, import_off] {
            out.extend_from_slice(&(v as i32).to_le_bytes());
        }
        out.extend_from_slice(&[0xAB; 16]);
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&4i32.to_le_bytes());
        assert_eq!(out.len(), header_len);
        out.extend_from_slice(&names);
        out.extend_from_slice(&imports);
        out.extend_from_slice(&exports);
        out.extend_from_slice(b"xyz");
        out
    }

    #[test]
    fn parses_tiny_package() {
        let p = Package::parse(tiny_package()).unwrap();
        assert_eq!((p.version, p.licensee), (128, 29));
        assert_eq!(p.names.len(), 4);
        assert_eq!(p.imports.len(), 1);
        assert_eq!(p.exports.len(), 1);
        assert_eq!(p.export_class_name(0), "Class");
        assert_eq!(p.object_name(ObjectRef::Export(0)), "Thing");
        assert_eq!(p.export_data(0), b"xyz");
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut bytes = tiny_package();
        bytes[0] = 0;
        assert!(Package::parse(bytes).is_err());
    }

    #[test]
    fn rejects_export_past_end() {
        let mut bytes = tiny_package();
        bytes.truncate(bytes.len() - 1);
        assert!(Package::parse(bytes).is_err());
    }

    #[test]
    fn object_ref_encoding() {
        assert_eq!(ObjectRef::from_raw(0), ObjectRef::Null);
        assert_eq!(ObjectRef::from_raw(1), ObjectRef::Export(0));
        assert_eq!(ObjectRef::from_raw(-1), ObjectRef::Import(0));
        assert_eq!(ObjectRef::from_raw(-532), ObjectRef::Import(531));
    }
}
