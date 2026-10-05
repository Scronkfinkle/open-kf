//! Unreal Engine 2 tagged properties: the name/type/value list at the start of
//! almost every object's data.
//!
//! Each property is stored as:
//! - name (compact index into the name table); the name `None` ends the list
//! - info byte: bits 0-3 type, bits 4-6 size code, bit 7 array flag
//!   (for booleans bit 7 is the value itself)
//! - struct name, for struct properties
//! - size, if the size code says it is stored separately
//! - array index, if the array flag is set
//! - the value, `size` bytes long
//!
//! Because the size is always known, values of types we cannot decode yet are
//! kept as raw bytes and the reader stays in step.

use std::fmt;

use crate::package::{ObjectRef, Package};
use crate::reader::{ReadError, ReadErrorKind, Reader, Result};

/// Object flag: the object's data starts with script-execution state.
pub const RF_HAS_STACK: u32 = 0x0200_0000;

/// A rotation in Unreal units, where 65536 is a full turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rotator {
    pub pitch: i32,
    pub yaw: i32,
    pub roll: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    Object(ObjectRef),
    Name(usize),
    Str(String),
    Vector([f32; 3]),
    Rotator(Rotator),
    /// Stored as B, G, R, A in the file; kept here as R, G, B, A.
    Color([u8; 4]),
    /// Any other struct: stored as its own nested tagged property list.
    TaggedStruct { name: usize, props: PropertyList },
    /// A struct that could not be read as a nested list; kept raw.
    Struct { name: usize, raw: Vec<u8> },
    /// A dynamic array. Element type is not stored in the file, so elements
    /// stay raw until a caller who knows the type decodes them.
    Array { count: usize, raw: Vec<u8> },
    /// An array the caller declared as holding structs. Each element is its
    /// own tagged property list.
    StructArray(Vec<PropertyList>),
    /// Any other type (delegates, maps, ...).
    Raw { type_id: u8, raw: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    /// Index into the package's name table.
    pub name: usize,
    pub array_index: u32,
    pub value: Value,
}

/// The properties of one object, plus where they ended.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyList {
    pub props: Vec<Property>,
    /// Byte offset (within the object's data) just after the terminating `None`.
    /// Native data such as texture pixels or mesh vertices starts here.
    pub end: usize,
    /// Nested structs whose real length exceeded their stored size (see
    /// "Stale property sizes" in DESIGN.md), counted for logging.
    pub stale_sizes: usize,
}

impl PropertyList {
    /// The first property with this name, at array index 0.
    pub fn get(&self, pkg: &Package, name: &str) -> Option<&Value> {
        self.get_at(pkg, name, 0)
    }

    /// Element `index` of a fixed-size array property (`name[index]`).
    pub fn get_at(&self, pkg: &Package, name: &str, index: u32) -> Option<&Value> {
        self.props
            .iter()
            .find(|p| p.array_index == index && pkg.name(p.name).eq_ignore_ascii_case(name))
            .map(|p| &p.value)
    }
}

// Property type ids in UE2.
const T_BYTE: u8 = 1;
const T_INT: u8 = 2;
const T_BOOL: u8 = 3;
const T_FLOAT: u8 = 4;
const T_OBJECT: u8 = 5;
const T_NAME: u8 = 6;
const T_CLASS: u8 = 8;
const T_ARRAY: u8 = 9;
const T_STRUCT: u8 = 10;
const T_STR: u8 = 13;

/// Export classes whose data is not a tagged property list (code, script
/// structure, property definitions).
pub fn has_tagged_properties(class_name: &str) -> bool {
    !matches!(
        class_name,
        "Class" | "Struct" | "Function" | "State" | "Enum" | "Const" | "TextBuffer"
    ) && !class_name.ends_with("Property")
}

/// Arrays known to hold structs, per class. These are decoded element by
/// element so a stale stored size cannot derail the reader.
pub fn known_struct_arrays(class_name: &str) -> &'static [&'static str] {
    match class_name {
        "StaticMesh" => &["Materials"],
        _ => &[],
    }
}

/// Reads the property list of export `index`.
pub fn read_export_properties(pkg: &Package, index: usize) -> Result<PropertyList> {
    read_export_properties_ext(pkg, index, known_struct_arrays(pkg.export_class_name(index)))
}

/// Like [`read_export_properties`], but arrays named in `struct_arrays` are
/// decoded element by element as tagged structs. Their stored size is ignored,
/// because it is sometimes too small (see "Stale property sizes" in DESIGN.md).
pub fn read_export_properties_ext(pkg: &Package, index: usize, struct_arrays: &[&str]) -> Result<PropertyList> {
    let data = pkg.export_data(index);
    let mut r = Reader::new(data);
    if pkg.exports[index].object_flags & RF_HAS_STACK != 0 {
        skip_state_frame(&mut r)?;
    }
    read_properties_ext(pkg, &mut r, struct_arrays)
}

/// Script execution state saved with some actors. Not used; only skipped.
fn skip_state_frame(r: &mut Reader) -> Result<()> {
    let node = r.compact_index()?;
    let _state_node = r.compact_index()?;
    let _probe_mask = r.bytes(8)?;
    let _latent_action = r.i32()?;
    if node != 0 {
        let _offset = r.compact_index()?;
    }
    Ok(())
}

/// Reads properties from the reader's current position up to and including `None`.
pub fn read_properties(pkg: &Package, r: &mut Reader) -> Result<PropertyList> {
    read_properties_ext(pkg, r, &[])
}

/// See [`read_export_properties_ext`].
pub fn read_properties_ext(pkg: &Package, r: &mut Reader, struct_arrays: &[&str]) -> Result<PropertyList> {
    let name_count = pkg.names.len();
    let mut props = Vec::new();
    let mut stale_sizes = 0usize;
    loop {
        let at = r.pos();
        let name = r.compact_index()?;
        if name < 0 || name as usize >= name_count {
            return Err(invalid(at, format!("property name index {name} out of range")));
        }
        let name = name as usize;
        if pkg.name(name).eq_ignore_ascii_case("None") {
            break;
        }

        let info = r.u8()?;
        let type_id = info & 0x0f;
        let size_code = (info >> 4) & 0x07;
        let array_flag = info & 0x80 != 0;

        let struct_name = if type_id == T_STRUCT {
            let at = r.pos();
            let n = r.compact_index()?;
            if n < 0 || n as usize >= name_count {
                return Err(invalid(at, format!("struct name index {n} out of range")));
            }
            Some(n as usize)
        } else {
            None
        };

        let size = match size_code {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 12,
            4 => 16,
            5 => r.u8()? as usize,
            6 => r.u16()? as usize,
            _ => {
                let at = r.pos();
                let n = r.i32()?;
                if n < 0 {
                    return Err(invalid(at, format!("negative property size {n}")));
                }
                n as usize
            }
        };

        let array_index = if array_flag && type_id != T_BOOL {
            read_array_index(r)?
        } else {
            0
        };

        if type_id == T_ARRAY && struct_arrays.iter().any(|n| pkg.name(name).eq_ignore_ascii_case(n)) {
            let at = r.pos();
            let count = r.compact_index()?;
            if !(0..=100_000).contains(&count) {
                return Err(invalid(at, format!("struct array count {count}")));
            }
            let mut elements = Vec::with_capacity(count as usize);
            for _ in 0..count {
                elements.push(read_properties(pkg, r)?);
            }
            props.push(Property {
                name,
                array_index,
                value: Value::StructArray(elements),
            });
            continue;
        }

        // Structs other than the three fixed-layout ones are nested tagged
        // lists. Read them by their own tags rather than the stored size,
        // which is sometimes too small (seen in TerrainLayer).
        if let Some(sn) = struct_name
            && !is_binary_struct(pkg.name(sn))
        {
            let start = r.pos();
            let declared_end = start + size;
            let nested = read_properties_ext(pkg, r, &[]);
            match nested {
                Ok(list) if r.pos() >= declared_end => {
                    if r.pos() > declared_end {
                        stale_sizes += 1;
                    }
                    stale_sizes += list.stale_sizes;
                    props.push(Property {
                        name,
                        array_index,
                        value: Value::TaggedStruct { name: sn, props: list },
                    });
                    continue;
                }
                _ => {
                    // Not a nested list after all: fall back to the stored size.
                    r.seek(start)?;
                }
            }
        }

        let value = if type_id == T_BOOL {
            // Booleans store their value in the array-flag bit and have no data.
            Value::Bool(array_flag)
        } else {
            let start = r.pos();
            let bytes = r.bytes(size)?;
            decode_value(pkg, type_id, struct_name, bytes).map_err(|e| ReadError {
                offset: start + e.offset,
                kind: e.kind,
            })?
        };

        props.push(Property { name, array_index, value });
    }
    Ok(PropertyList {
        props,
        end: r.pos(),
        stale_sizes,
    })
}

/// Structs stored as fixed binary data inside tagged properties. Every other
/// struct is a nested tagged list. Determined from the data: across four
/// packages, Vector, Rotator and Color never parse as tagged lists, while
/// Scale, Plane, Range, RangeVector, PointRegion and TerrainLayer always do.
fn is_binary_struct(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "vector" | "rotator" | "color")
}

/// Array index: 1, 2 or 4 bytes, length given by the top bits of the first byte.
fn read_array_index(r: &mut Reader) -> Result<u32> {
    let b0 = r.u8()? as u32;
    Ok(if b0 & 0x80 == 0 {
        b0
    } else if b0 & 0xc0 == 0x80 {
        ((b0 & 0x7f) << 8) | r.u8()? as u32
    } else {
        let rest = r.bytes(3)?;
        ((b0 & 0x3f) << 24) | (rest[0] as u32) << 16 | (rest[1] as u32) << 8 | rest[2] as u32
    })
}

fn decode_value(pkg: &Package, type_id: u8, struct_name: Option<usize>, bytes: &[u8]) -> Result<Value> {
    let mut r = Reader::new(bytes);
    let value = match type_id {
        T_BYTE if bytes.len() == 1 => Value::Byte(r.u8()?),
        T_INT if bytes.len() == 4 => Value::Int(r.i32()?),
        T_FLOAT if bytes.len() == 4 => Value::Float(r.f32()?),
        T_OBJECT | T_CLASS => {
            let rf = ObjectRef::from_raw(r.compact_index()?);
            let ok = match rf {
                ObjectRef::Null => true,
                ObjectRef::Export(i) => i < pkg.exports.len(),
                ObjectRef::Import(i) => i < pkg.imports.len(),
            };
            if !ok {
                return Err(r.error(ReadErrorKind::Invalid(format!("object reference {rf:?} out of range"))));
            }
            Value::Object(rf)
        }
        T_NAME => {
            let n = r.compact_index()?;
            if n < 0 || n as usize >= pkg.names.len() {
                return Err(r.error(ReadErrorKind::Invalid(format!("name index {n} out of range"))));
            }
            Value::Name(n as usize)
        }
        T_STR => Value::Str(r.fstring()?),
        T_ARRAY => {
            let count = r.compact_index()?;
            Value::Array {
                count: count.max(0) as usize,
                raw: bytes[r.pos()..].to_vec(),
            }
        }
        T_STRUCT => {
            // decode_struct reads `bytes` itself, so return before the unread check below.
            let name = struct_name.expect("struct properties always have a struct name");
            return decode_struct(pkg.name(name), name, bytes);
        }
        _ => {
            return Ok(Value::Raw {
                type_id,
                raw: bytes.to_vec(),
            });
        }
    };
    if r.remaining() != 0 && !matches!(value, Value::Array { .. }) {
        return Err(r.error(ReadErrorKind::Invalid(format!(
            "value of type {type_id} left {} of {} bytes unread",
            r.remaining(),
            bytes.len()
        ))));
    }
    Ok(value)
}

fn decode_struct(struct_name: &str, name: usize, bytes: &[u8]) -> Result<Value> {
    let mut r = Reader::new(bytes);
    let value = match (struct_name.to_ascii_lowercase().as_str(), bytes.len()) {
        ("vector", 12) => Value::Vector([r.f32()?, r.f32()?, r.f32()?]),
        ("rotator", 12) => Value::Rotator(Rotator {
            pitch: r.i32()?,
            yaw: r.i32()?,
            roll: r.i32()?,
        }),
        ("color", 4) => {
            let [b, g, red, a] = [r.u8()?, r.u8()?, r.u8()?, r.u8()?];
            Value::Color([red, g, b, a])
        }
        _ => Value::Struct {
            name,
            raw: bytes.to_vec(),
        },
    };
    Ok(value)
}

fn invalid(offset: usize, msg: String) -> ReadError {
    ReadError {
        offset,
        kind: ReadErrorKind::Invalid(msg),
    }
}

/// Formats a value for humans, resolving names and object paths.
pub struct DisplayValue<'a>(pub &'a Package, pub &'a Value);

impl fmt::Display for DisplayValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pkg = self.0;
        match self.1 {
            Value::Byte(v) => write!(f, "{v}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v}"),
            Value::Object(rf) => write!(f, "{}", pkg.object_path(*rf)),
            Value::Name(n) => write!(f, "'{}'", pkg.name(*n)),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Vector([x, y, z]) => write!(f, "(X={x}, Y={y}, Z={z})"),
            Value::Rotator(rot) => write!(f, "(Pitch={}, Yaw={}, Roll={})", rot.pitch, rot.yaw, rot.roll),
            Value::Color([r, g, b, a]) => write!(f, "(R={r}, G={g}, B={b}, A={a})"),
            Value::TaggedStruct { props, .. } => {
                write!(f, "(")?;
                for (j, p) in props.props.iter().enumerate() {
                    if j > 0 {
                        write!(f, ", ")?;
                    }
                    let idx = if p.array_index > 0 { format!("[{}]", p.array_index) } else { String::new() };
                    write!(f, "{}{idx}={}", pkg.name(p.name), DisplayValue(pkg, &p.value))?;
                }
                write!(f, ")")
            }
            Value::Struct { name, raw } => write!(f, "<struct {} {} bytes>", pkg.name(*name), raw.len()),
            Value::Array { count, raw } => write!(f, "<array {count} items, {} bytes>", raw.len()),
            Value::StructArray(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "(")?;
                    for (j, p) in item.props.iter().enumerate() {
                        if j > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}={}", pkg.name(p.name), DisplayValue(pkg, &p.value))?;
                    }
                    write!(f, ")")?;
                }
                write!(f, "]")
            }
            Value::Raw { type_id, raw } => write!(f, "<type {type_id}, {} bytes>", raw.len()),
        }
    }
}

/// Finds and reads a class's default property values.
///
/// A class export stores its script (compiled bytecode, whose serialized
/// length is not recorded) before the defaults, so the defaults are located
/// from the end: the first offset from which a property list parses and ends
/// exactly at the end of the data, with every property name in
/// `property_names` (names of property objects across the game's code).
/// Returns the list and the offset where it starts.
/// Adds the (lowercase) names of the properties a package's classes declare,
/// for `find_class_defaults`. Struct members (e.g. Range's `Min` / `Max`) are
/// left out: they never appear at the top level of a class's defaults, and
/// accepting them let a false start parse as valid (KFMod.SingleFire).
pub fn add_class_property_names(pkg: &Package, names: &mut std::collections::HashSet<String>) {
    for i in 0..pkg.exports.len() {
        let declared_by_class = match pkg.exports[i].outer {
            ObjectRef::Export(o) => pkg.export_class_name(o) == "Class",
            _ => false,
        };
        if declared_by_class && pkg.export_class_name(i).ends_with("Property") {
            names.insert(pkg.object_name(ObjectRef::Export(i)).to_ascii_lowercase());
        }
    }
}

pub fn find_class_defaults(
    pkg: &Package,
    export: usize,
    property_names: &std::collections::HashSet<String>,
) -> Option<(PropertyList, usize)> {
    // Every start offset whose properties read exactly to the end, with
    // only known property names, is a candidate. The earliest is not always
    // right: a false start can read as a few bogus properties that swallow
    // the real ones (e.g. BullpupAmmo: from byte 66, a "Range" of type 14
    // and 43 bytes; the real list starts at 93), or that add a bogus first
    // value (often "User" of type 0). Bogus values come out as Raw: type 0,
    // a map or fixed array (14, 15), or a wrong size for their type.
    // Delegates (type 7) are also kept Raw but are real. So the candidate
    // with the fewest non-delegate Raw values wins, then the one with the
    // most properties, then the earliest.
    let data = pkg.export_data(export);
    let raw_count = |l: &PropertyList| {
        l.props
            .iter()
            .filter(|p| matches!(p.value, Value::Raw { type_id, .. } if type_id != 7))
            .count()
    };
    let mut best: Option<(PropertyList, usize)> = None;
    for start in 0..data.len() {
        let mut r = Reader::new(data);
        if r.seek(start).is_err() {
            continue;
        }
        let Ok(list) = read_properties(pkg, &mut r) else {
            continue;
        };
        if r.pos() != data.len() || list.props.is_empty() {
            continue;
        }
        if list
            .props
            .iter()
            .all(|p| property_names.contains(&pkg.name(p.name).to_ascii_lowercase()))
            && best
                .as_ref()
                .is_none_or(|(b, _)| (raw_count(&list), std::cmp::Reverse(list.props.len())) < (raw_count(b), std::cmp::Reverse(b.props.len())))
        {
            best = Some((list, start));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_index_lengths() {
        assert_eq!(read_array_index(&mut Reader::new(&[5])).unwrap(), 5);
        assert_eq!(read_array_index(&mut Reader::new(&[0x81, 0x02])).unwrap(), 0x102);
        assert_eq!(
            read_array_index(&mut Reader::new(&[0xc1, 0x02, 0x03, 0x04])).unwrap(),
            0x0102_0304
        );
    }

    #[test]
    fn known_structs_decode() {
        let mut v = Vec::new();
        for f in [1.0f32, -2.5, 3.0] {
            v.extend_from_slice(&f.to_le_bytes());
        }
        assert_eq!(decode_struct("Vector", 0, &v).unwrap(), Value::Vector([1.0, -2.5, 3.0]));
        // Color is stored B, G, R, A.
        assert_eq!(
            decode_struct("Color", 0, &[10, 20, 30, 255]).unwrap(),
            Value::Color([30, 20, 10, 255])
        );
        // Unknown struct stays raw rather than failing.
        assert!(matches!(decode_struct("Mystery", 7, &[1, 2]).unwrap(), Value::Struct { name: 7, .. }));
    }
}
