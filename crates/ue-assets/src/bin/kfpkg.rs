//! `kfpkg`: inspect Killing Floor packages.
//!
//! ```text
//! kfpkg scan                       parse every package in the install
//! kfpkg info <file>                header and object counts by class
//! kfpkg exports <file> [CLASS]     list exported objects, optionally of one class
//! kfpkg props <file> [CLASS]       write object properties to work/props/
//! kfpkg scanprops                  read the properties of every object in the install
//! kfpkg scripts                    write all UnrealScript source to work/scripts/
//! kfpkg textures                   decode every texture in the install (checks only)
//! kfpkg texture <file> <name>      write one texture's first mip to work/textures/ as PNG
//! kfpkg meshes                     decode every static mesh in the install (checks only)
//! kfpkg level <map>                load a map like the viewer does and report counts
//! kfpkg bspmaterials <map>         per-material report of the level geometry -> logs/
//! kfpkg zones <map> [ZONE]         BSP zones: owning actor, polygons, backdrop polygons;
//!                                  with ZONE, list every polygon touching that zone
//! kfpkg terrain <map> [X,Y]        terrain grid, layers, a height check against ground actors,
//!                                  and optionally the ground height at Unreal X,Y
//! kfpkg defaults <Package.Class>   decoded default properties of a class (e.g. Engine.Pawn)
//! kfpkg polys                      read every Polys object in the install (checks only)
//! kfpkg brush <map> <ActorName>    a brush actor's polygon bounds (local, and world both ways)
//! kfpkg skelmeshes                 read every skeletal mesh (checks only)
//! kfpkg anims                      read every animation set (checks only)
//! kfpkg meshtags <file> <mesh>     a skeletal mesh's bones and attach tags
//! kfpkg emitter <Package.Class>    a particle effect's sub-emitters, values resolved
//! kfpkg nav <map>                  a map's navigation network: nodes, ReachSpec flags, groups
//! ```
//! `<file>` may be absolute or relative to the install, e.g. `Maps/KF-Farm.rom`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{LineWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ue_assets::install::Install;
use ue_assets::package::{MAGIC_KILLING_FLOOR, ObjectRef, Package};
use ue_assets::properties::{DisplayValue, has_tagged_properties, read_export_properties};
use ue_assets::texture::{Texture, decode_rgba, read_palette, read_texture};

/// Folders and extensions that hold Unreal packages.
const PACKAGE_DIRS: &[(&str, &str)] = &[
    ("System", "u"),
    ("Maps", "rom"),
    ("Textures", "utx"),
    ("StaticMeshes", "usx"),
    ("Animations", "ukx"),
    ("Sounds", "uax"),
];

const USAGE: &str = "usage:
  kfpkg scan
  kfpkg info <file>
  kfpkg exports <file> [CLASS]
  kfpkg props <file> [CLASS]
  kfpkg scanprops
  kfpkg scripts
  kfpkg textures
  kfpkg texture <file> <name>
  kfpkg meshes
  kfpkg level <map>
  kfpkg bspmaterials <map>
  kfpkg zones <map> [ZONE]
  kfpkg terrain <map> [X,Y]
  kfpkg defaults <Package.Class>
  kfpkg polys
  kfpkg brush <map> <ActorName>
  kfpkg skelmeshes
  kfpkg anims
  kfpkg karma
  kfpkg meshtags <file> <mesh>
  kfpkg emitter <Package.Class>
  kfpkg nav <map>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let install = match Install::discover() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["scan"] => scan(&install),
        ["info", file] => info(&install, file),
        ["exports", file] => exports(&install, file, None),
        ["exports", file, class] => exports(&install, file, Some(class)),
        ["props", file] => props(&install, file, None),
        ["props", file, class] => props(&install, file, Some(class)),
        ["scanprops"] => scan_props(&install),
        ["scripts"] => scripts(&install),
        ["textures"] => scan_textures(&install),
        ["texture", file, name] => export_texture(&install, file, name),
        ["meshes"] => scan_meshes(&install),
        ["level", map] => level(&install, map),
        ["bspmaterials", map] => bsp_materials(&install, map),
        ["zones", map] => zones(&install, map),
        ["terrain", map] => terrain(&install, map, None),
        ["terrain", map, at] => terrain(&install, map, Some(at)),
        ["defaults", class] => class_defaults(&install, class),
        ["polys"] => scan_polys(&install),
        ["brush", map, actor] => brush_bounds(&install, map, actor),
        ["skelmeshes"] => scan_skeletal(&install),
        ["anims"] => scan_anims(&install),
        ["karma"] => scan_karma(&install),
        ["emitter", class] => emitter(&install, class),
        ["nav", map] => nav(&install, map),
        ["meshtags", file, mesh] => mesh_tags(&install, file, mesh),
        ["notifies", file, anim] => notifies(&install, file, anim),
        ["zones", map, zone] => zone_polygons(&install, map, zone.parse().map_err(|_| "bad zone").unwrap_or(0)),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn resolve(install: &Install, file: &str) -> PathBuf {
    let p = Path::new(file);
    if p.exists() { p.to_path_buf() } else { install.root.join(p) }
}

fn open(install: &Install, file: &str) -> Result<Package, String> {
    let path = resolve(install, file);
    Package::open(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn magic_label(p: &Package) -> &'static str {
    if p.magic == MAGIC_KILLING_FLOOR { "kf" } else { "standard" }
}

/// Parses every package. Prints failures and a summary; writes one line per
/// file to `logs/kfpkg-scan.log`. Returns false if any file failed.
fn scan(install: &Install) -> Result<bool, String> {
    let log_path = "logs/kfpkg-scan.log";
    let mut log = create_log(log_path)?;
    let started = std::time::Instant::now();

    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut bytes = 0u64;
    let (mut names, mut imports, mut exports) = (0usize, 0usize, 0usize);
    let mut per_dir: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();

    for (dir, path) in package_files(install)? {
        {
            let rel = path.strip_prefix(&install.root).unwrap_or(&path).display().to_string();
            let entry = per_dir.entry(dir).or_default();
            match Package::open(&path) {
                Ok(p) => {
                    ok += 1;
                    entry.0 += 1;
                    bytes += p.data().len() as u64;
                    names += p.names.len();
                    imports += p.imports.len();
                    exports += p.exports.len();
                    *versions
                        .entry(format!("magic={} version={} licensee={}", magic_label(&p), p.version, p.licensee))
                        .or_default() += 1;
                    let _ = writeln!(
                        log,
                        "ok file=\"{rel}\" magic={} version={} licensee={} names={} imports={} exports={} bytes={}",
                        magic_label(&p),
                        p.version,
                        p.licensee,
                        p.names.len(),
                        p.imports.len(),
                        p.exports.len(),
                        p.data().len()
                    );
                }
                Err(e) => {
                    failed += 1;
                    entry.1 += 1;
                    println!("FAIL {rel}: {e}");
                    let _ = writeln!(log, "fail file=\"{rel}\" error=\"{e}\"");
                }
            }
        }
    }

    let secs = started.elapsed().as_secs_f64();
    let mut summary = vec![format!(
        "summary files={} ok={ok} failed={failed} names={names} imports={imports} exports={exports} megabytes={:.0} seconds={secs:.1}",
        ok + failed,
        bytes as f64 / 1e6
    )];
    for (dir, (o, f)) in &per_dir {
        summary.push(format!("dir={dir} ok={o} failed={f}"));
    }
    for (v, n) in &versions {
        summary.push(format!("{v} files={n}"));
    }
    for line in &summary {
        println!("{line}");
        let _ = writeln!(log, "{line}");
    }
    println!("per-file details: {log_path}");
    Ok(failed == 0)
}

fn create_log(path: &str) -> Result<LineWriter<File>, String> {
    std::fs::create_dir_all("logs").map_err(|e| e.to_string())?;
    Ok(LineWriter::new(File::create(path).map_err(|e| format!("{path}: {e}"))?))
}

/// Every package in the install, as (folder, path), sorted within each folder.
fn package_files(install: &Install) -> Result<Vec<(&'static str, PathBuf)>, String> {
    let mut out = Vec::new();
    for &(dir, ext) in PACKAGE_DIRS {
        let mut files: Vec<PathBuf> = std::fs::read_dir(install.root.join(dir))
            .map_err(|e| format!("{dir}: {e}"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
            .collect();
        files.sort();
        out.extend(files.into_iter().map(|p| (dir, p)));
    }
    Ok(out)
}

fn info(install: &Install, file: &str) -> Result<bool, String> {
    let p = open(install, file)?;
    println!(
        "magic={} version={} licensee={} flags={:#x} bytes={}",
        magic_label(&p),
        p.version,
        p.licensee,
        p.flags,
        p.data().len()
    );
    println!("names={} imports={} exports={}", p.names.len(), p.imports.len(), p.exports.len());
    for (i, g) in p.generations.iter().enumerate() {
        println!("generation {i}: exports={} names={}", g.export_count, g.name_count);
    }
    let mut classes: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for i in 0..p.exports.len() {
        let c = classes.entry(p.export_class_name(i)).or_default();
        c.0 += 1;
        c.1 += p.exports[i].serial_size;
    }
    let mut classes: Vec<_> = classes.into_iter().collect();
    classes.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    println!("exports by class (count, total bytes):");
    let mut out = std::io::stdout().lock();
    for (name, (count, size)) in classes {
        // Stop quietly if the reader went away (e.g. piped into `head`).
        if writeln!(out, "  {count:>7}  {size:>10}  {name}").is_err() {
            break;
        }
    }
    Ok(true)
}

fn exports(install: &Install, file: &str, class: Option<&str>) -> Result<bool, String> {
    let p = open(install, file)?;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{:>7}  {:<24}  {:>9}  path", "index", "class", "bytes");
    for (i, e) in p.exports.iter().enumerate() {
        let cname = p.export_class_name(i);
        if class.is_some_and(|c| !c.eq_ignore_ascii_case(cname)) {
            continue;
        }
        let line = writeln!(
            out,
            "{i:>7}  {cname:<24}  {:>9}  {}",
            e.serial_size,
            p.object_path(ObjectRef::Export(i))
        );
        // Stop quietly if the reader went away (e.g. piped into `head`).
        if line.is_err() {
            break;
        }
    }
    Ok(true)
}

/// Writes the properties of every export (or every export of one class) to
/// `work/props/<package>[-<class>].txt` and prints a short summary.
fn props(install: &Install, file: &str, class: Option<&str>) -> Result<bool, String> {
    let p = open(install, file)?;
    let stem = Path::new(file).file_stem().map_or("package".into(), |s| s.to_string_lossy().into_owned());
    let out_name = match class {
        Some(c) => format!("{stem}-{c}.txt"),
        None => format!("{stem}.txt"),
    };
    std::fs::create_dir_all("work/props").map_err(|e| e.to_string())?;
    let out_path = Path::new("work/props").join(out_name);
    let mut out = std::io::BufWriter::new(File::create(&out_path).map_err(|e| e.to_string())?);

    let (mut objects, mut failed, mut props_total) = (0usize, 0usize, 0usize);
    for i in 0..p.exports.len() {
        let cname = p.export_class_name(i);
        if class.is_some_and(|c| !c.eq_ignore_ascii_case(cname)) || !has_tagged_properties(cname) {
            continue;
        }
        objects += 1;
        let path = p.object_path(ObjectRef::Export(i));
        let w = |out: &mut std::io::BufWriter<File>, s: String| writeln!(out, "{s}").map_err(|e| e.to_string());
        match read_export_properties(&p, i) {
            Ok(list) => {
                props_total += list.props.len();
                w(
                    &mut out,
                    format!(
                        "[{i}] {cname} {path}  (props end at byte {} of {})",
                        list.end, p.exports[i].serial_size
                    ),
                )?;
                for prop in &list.props {
                    let idx = if prop.array_index > 0 { format!("[{}]", prop.array_index) } else { String::new() };
                    w(&mut out, format!("    {}{idx} = {}", p.name(prop.name), DisplayValue(&p, &prop.value)))?;
                }
            }
            Err(e) => {
                failed += 1;
                w(
                    &mut out,
                    format!(
                        "[{i}] {cname} {path}  FAILED: {e} (object data at file offset {}, {} bytes)",
                        p.exports[i].serial_offset, p.exports[i].serial_size
                    ),
                )?;
            }
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    println!(
        "objects={objects} failed={failed} properties={props_total} written={}",
        out_path.display()
    );
    Ok(failed == 0)
}

#[derive(Default)]
struct ClassStats {
    ok: usize,
    failed: usize,
    /// Property list ended exactly at the end of the object's data.
    exact_end: usize,
    first_error: Option<String>,
}

/// Reads the properties of every object in every package. Prints failures by
/// class and a summary; writes per-class numbers to `logs/kfpkg-props.log`.
fn scan_props(install: &Install) -> Result<bool, String> {
    let log_path = "logs/kfpkg-props.log";
    let mut log = create_log(log_path)?;
    let started = std::time::Instant::now();
    let mut stats: BTreeMap<String, ClassStats> = BTreeMap::new();
    let mut skipped = 0usize;

    for (_, path) in package_files(install)? {
        let rel = path.strip_prefix(&install.root).unwrap_or(&path).display().to_string();
        let p = Package::open(&path).map_err(|e| format!("{rel}: {e}"))?;
        for i in 0..p.exports.len() {
            let cname = p.export_class_name(i);
            if !has_tagged_properties(cname) {
                skipped += 1;
                continue;
            }
            let s = stats.entry(cname.to_string()).or_default();
            match read_export_properties(&p, i) {
                Ok(list) => {
                    s.ok += 1;
                    if list.end == p.exports[i].serial_size {
                        s.exact_end += 1;
                    }
                }
                Err(e) => {
                    s.failed += 1;
                    if s.first_error.is_none() {
                        s.first_error = Some(format!("{rel} export {i} {}: {e}", p.object_path(ObjectRef::Export(i))));
                    }
                }
            }
        }
    }

    let (mut ok, mut failed) = (0usize, 0usize);
    for (class, s) in &stats {
        ok += s.ok;
        failed += s.failed;
        let _ = writeln!(
            log,
            "class={class} ok={} failed={} exact_end={}",
            s.ok, s.failed, s.exact_end
        );
        if let Some(e) = &s.first_error {
            println!("FAIL class={class} failed={} first: {e}", s.failed);
            let _ = writeln!(log, "first_error class={class} {e}");
        }
    }
    let summary = format!(
        "summary objects={} ok={ok} failed={failed} classes={} skipped_code_objects={skipped} seconds={:.1}",
        ok + failed,
        stats.len(),
        started.elapsed().as_secs_f64()
    );
    println!("{summary}");
    let _ = writeln!(log, "{summary}");
    println!("per-class details: {log_path}");
    Ok(failed == 0)
}

/// Writes every class's source text to `work/scripts/<Package>/<Class>.uc`.
/// Returns false if any text failed to read or a class has no source.
fn scripts(install: &Install) -> Result<bool, String> {
    let mut all_ok = true;
    let (mut classes_total, mut written_total, mut bytes_total) = (0usize, 0usize, 0usize);
    for (dir, path) in package_files(install)? {
        if dir != "System" {
            continue;
        }
        let stem = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        let p = Package::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let classes = (0..p.exports.len()).filter(|&i| p.export_class_name(i) == "Class").count();
        let (sources, failed) = ue_assets::script_text::read_all(&p);
        let out_dir = Path::new("work/scripts").join(&stem);
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let mut empty = 0usize;
        for s in &sources {
            if s.text.trim().is_empty() {
                empty += 1;
            }
            bytes_total += s.text.len();
            std::fs::write(out_dir.join(format!("{}.uc", s.class_name)), &s.text).map_err(|e| e.to_string())?;
        }
        for (i, e) in &failed {
            println!("FAIL {stem} export {i}: {e}");
        }
        let missing = classes.saturating_sub(sources.len());
        if !failed.is_empty() || missing > 0 {
            all_ok = false;
        }
        println!(
            "package={stem} classes={classes} sources={} empty={empty} missing={missing} failed={}",
            sources.len(),
            failed.len()
        );
        classes_total += classes;
        written_total += sources.len();
    }
    println!(
        "summary classes={classes_total} sources_written={written_total} megabytes={:.1} dir=work/scripts",
        bytes_total as f64 / 1e6
    );
    Ok(all_ok)
}

/// Texture classes: `Texture` and its subclasses that carry pixel data.
fn is_texture_class(class_name: &str) -> bool {
    matches!(class_name, "Texture" | "Cubemap")
}

/// Palette for a P8 texture, if it lives in the same package.
fn local_palette(p: &Package, t: &Texture) -> Option<Vec<[u8; 4]>> {
    match t.palette_ref {
        ObjectRef::Export(i) => read_palette(p, i).ok(),
        _ => None,
    }
}

/// Decodes every texture in every package and reports formats and problems.
fn scan_textures(install: &Install) -> Result<bool, String> {
    let log_path = "logs/kfpkg-textures.log";
    let mut log = create_log(log_path)?;
    let started = std::time::Instant::now();
    let mut formats: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // (textures, decoded)
    let (mut total, mut read_failed, mut size_mismatch, mut trailing, mut no_mips) = (0usize, 0, 0, 0, 0);
    let mut examples: BTreeMap<&str, String> = BTreeMap::new();

    for (_, path) in package_files(install)? {
        let rel = path.strip_prefix(&install.root).unwrap_or(&path).display().to_string();
        let p = Package::open(&path).map_err(|e| format!("{rel}: {e}"))?;
        for i in 0..p.exports.len() {
            if !is_texture_class(p.export_class_name(i)) {
                continue;
            }
            total += 1;
            let name = p.object_path(ObjectRef::Export(i));
            let t = match read_texture(&p, i) {
                Ok(t) => t,
                Err(e) => {
                    read_failed += 1;
                    let msg = format!("read_failed file=\"{rel}\" texture={name} error=\"{e}\"");
                    let _ = writeln!(log, "{msg}");
                    examples.entry("read_failed").or_insert(msg);
                    continue;
                }
            };
            let f = formats.entry(format!("{:?}", t.format)).or_default();
            f.0 += 1;
            if t.trailing_bytes > 0 {
                trailing += 1;
                let msg = format!("trailing file=\"{rel}\" texture={name} bytes={}", t.trailing_bytes);
                let _ = writeln!(log, "{msg}");
                examples.entry("trailing").or_insert(msg);
            }
            let Some(mip0) = t.mips.first() else {
                no_mips += 1;
                continue;
            };
            let bad_mip = t.mips.iter().enumerate().find(|(_, m)| {
                t.format.data_size(m.width, m.height).is_some_and(|n| n != m.data.len())
            });
            if let Some((mi, m)) = bad_mip {
                size_mismatch += 1;
                let msg = format!(
                    "size_mismatch file=\"{rel}\" texture={name} format={:?} mip0={}x{} mips={} first_bad_mip={mi} size={}x{} bytes={} expected={}",
                    t.format,
                    mip0.width,
                    mip0.height,
                    t.mips.len(),
                    m.width,
                    m.height,
                    m.data.len(),
                    t.format.data_size(m.width, m.height).unwrap_or(0)
                );
                let _ = writeln!(log, "{msg}");
                examples.entry("size_mismatch").or_insert(msg);
            }
            let palette = local_palette(&p, &t);
            if decode_rgba(t.format, mip0, palette.as_deref()).is_some() {
                f.1 += 1;
            }
        }
    }

    let mut lines = vec![format!(
        "summary textures={total} read_failed={read_failed} no_mips={no_mips} size_mismatch={size_mismatch} trailing_bytes={trailing} seconds={:.1}",
        started.elapsed().as_secs_f64()
    )];
    for (f, (n, ok)) in &formats {
        lines.push(format!("format={f} textures={n} decoded={ok}"));
    }
    for (kind, e) in &examples {
        lines.push(format!("first {kind}: {e}"));
    }
    for l in &lines {
        println!("{l}");
        let _ = writeln!(log, "{l}");
    }
    println!("details: {log_path}");
    Ok(read_failed == 0)
}

/// Writes the first mip of one texture to `work/textures/<name>.png`.
/// `name` matches the object name or the full dotted path, ignoring case.
fn export_texture(install: &Install, file: &str, name: &str) -> Result<bool, String> {
    let p = open(install, file)?;
    let index = (0..p.exports.len())
        .find(|&i| {
            is_texture_class(p.export_class_name(i))
                && (p.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(name)
                    || p.object_path(ObjectRef::Export(i)).eq_ignore_ascii_case(name))
        })
        .ok_or_else(|| format!("no texture named {name} in {file}"))?;
    let t = read_texture(&p, index).map_err(|e| e.to_string())?;
    let mip = t.mips.first().ok_or("texture has no mips")?;
    let palette = local_palette(&p, &t);
    let rgba = decode_rgba(t.format, mip, palette.as_deref())
        .ok_or_else(|| format!("cannot decode format {:?} yet", t.format))?;
    std::fs::create_dir_all("work/textures").map_err(|e| e.to_string())?;
    let out_path = Path::new("work/textures").join(format!("{}.png", p.object_path(ObjectRef::Export(index))));
    let file_out = File::create(&out_path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file_out), mip.width as u32, mip.height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&rgba))
        .map_err(|e| e.to_string())?;
    println!(
        "format={:?} size={}x{} mips={} written={}",
        t.format,
        mip.width,
        mip.height,
        t.mips.len(),
        out_path.display()
    );
    // Average colour of the decoded image, to compare with the stored MipZero
    // (the engine's own average colour) as a check on channel order.
    let mut sum = [0u64; 4];
    for px in rgba.as_chunks::<4>().0 {
        for c in 0..4 {
            sum[c] += px[c] as u64;
        }
    }
    let n = (rgba.len() / 4).max(1) as u64;
    println!(
        "decoded_average=(R={}, G={}, B={}, A={}) stored_MipZero={}",
        sum[0] / n,
        sum[1] / n,
        sum[2] / n,
        sum[3] / n,
        t.props
            .get(&p, "MipZero")
            .map_or("none".to_string(), |v| DisplayValue(&p, v).to_string())
    );
    Ok(true)
}

/// Decodes every static mesh and checks it for consistency.
fn scan_meshes(install: &Install) -> Result<bool, String> {
    let log_path = "logs/kfpkg-meshes.log";
    let mut log = create_log(log_path)?;
    let started = std::time::Instant::now();
    let (mut total, mut failed, mut outside_meshes, mut no_material) = (0usize, 0usize, 0usize, 0usize);
    let (mut verts, mut tris, mut material_mismatch) = (0usize, 0usize, 0usize);
    let mut first_errors: Vec<String> = Vec::new();

    for (_, path) in package_files(install)? {
        let rel = path.strip_prefix(&install.root).unwrap_or(&path).display().to_string();
        let p = Package::open(&path).map_err(|e| format!("{rel}: {e}"))?;
        for i in 0..p.exports.len() {
            if p.export_class_name(i) != "StaticMesh" {
                continue;
            }
            total += 1;
            let name = p.object_path(ObjectRef::Export(i));
            match ue_assets::static_mesh::read_static_mesh(&p, i) {
                Ok(m) => {
                    let outside = m.vertices_outside_bounds();
                    if outside > 0 {
                        outside_meshes += 1;
                    }
                    if m.materials.is_empty() {
                        no_material += 1;
                    }
                    if m.materials.len() != m.sections.len() {
                        material_mismatch += 1;
                    }
                    let t: usize = m.sections.iter().map(|s| s.num_triangles).sum();
                    verts += m.positions.len();
                    tris += t;
                    let _ = writeln!(
                        log,
                        "ok file=\"{rel}\" mesh={name} verts={} tris={t} sections={} materials={} uv_sets={} outside_bounds={outside} min={:?} max={:?}",
                        m.positions.len(),
                        m.sections.len(),
                        m.materials.len(),
                        m.uvs.len(),
                        m.bounds.min,
                        m.bounds.max
                    );
                }
                Err(e) => {
                    failed += 1;
                    let msg = format!("fail file=\"{rel}\" mesh={name} error=\"{e}\"");
                    let _ = writeln!(log, "{msg}");
                    if first_errors.len() < 10 {
                        first_errors.push(msg);
                    }
                }
            }
        }
    }
    for e in &first_errors {
        println!("{e}");
    }
    let summary = format!(
        "summary meshes={total} ok={} failed={failed} vertices={verts} triangles={tris} meshes_with_vertices_outside_bounds={outside_meshes} no_materials={no_material} materials_ne_sections={material_mismatch} seconds={:.1}",
        total - failed,
        started.elapsed().as_secs_f64()
    );
    println!("{summary}");
    let _ = writeln!(log, "{summary}");
    println!("details: {log_path}");
    Ok(failed == 0)
}

/// Loads a map the way the viewer will and reports what it found.
fn level(install: &Install, map: &str) -> Result<bool, String> {
    use std::collections::{BTreeMap, HashMap};
    use ue_assets::bsp::{poly_flags, read_model};
    use ue_assets::material::{Blend, resolve};
    use ue_assets::package_set::{ObjectHandle, PackageSet};

    let started = std::time::Instant::now();
    let file = if map.contains('/') || map.contains('.') { map.to_string() } else { format!("Maps/{map}.rom") };
    let set = PackageSet::new(&install.root);
    let lp = set.load_path(&resolve_path(install, &file)).map_err(|e| e.to_string())?;
    let pkg = &lp.pkg;
    let class_defaults = ue_assets::class_defaults::ClassDefaults::new(&set);
    let contents = ue_assets::level::read_level_with(&lp, &class_defaults);
    let map_handle = |export| ObjectHandle { package: lp.clone(), export };
    let blocking = contents.mesh_actors.iter().filter(|a| a.blocks_player).count();
    let mut brush_classes: BTreeMap<&str, usize> = BTreeMap::new();
    for b in &contents.blocking_brushes {
        *brush_classes.entry(b.class.as_str()).or_default() += 1;
    }
    println!(
        "collision: mesh actors blocking={blocking} of {} blocking_brushes={brush_classes:?} class_defaults_not_found={:?}",
        contents.mesh_actors.len(),
        class_defaults.not_found.borrow()
    );

    println!(
        "bsp_model={:?} bsp_candidates={} mesh_actors={} player_starts={} unreadable_objects={}",
        contents.bsp_model.map(|m| pkg.object_path(ObjectRef::Export(m))),
        contents.bsp_candidates,
        contents.mesh_actors.len(),
        contents.player_starts.len(),
        contents.unreadable
    );

    // BSP
    let mut bsp_ok = true;
    let mut materials: HashMap<String, (usize, ue_assets::material::SimpleMaterial)> = HashMap::new();
    if let Some(m) = contents.bsp_model {
        match read_model(pkg, m) {
            Ok(model) => {
                let (mut drawn, mut tris) = (0usize, 0usize);
                let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for (i, n) in model.nodes.iter().enumerate() {
                    let s = &model.surfs[n.surf];
                    let reason = if s.flags & poly_flags::INVISIBLE != 0 {
                        Some("invisible")
                    } else if s.flags & poly_flags::PORTAL != 0 {
                        Some("portal")
                    } else if s.flags & poly_flags::FAKE_BACKDROP != 0 {
                        Some("sky_backdrop")
                    } else {
                        None
                    };
                    if let Some(r) = reason {
                        *skipped.entry(r).or_default() += 1;
                        continue;
                    }
                    drawn += 1;
                    tris += n.num_verts.saturating_sub(2);
                    for p in model.node_polygon(i) {
                        for a in 0..3 {
                            lo[a] = lo[a].min(p[a]);
                            hi[a] = hi[a].max(p[a]);
                        }
                    }
                    let key = format!("bsp:{:?}", s.material);
                    let entry = materials
                        .entry(key)
                        .or_insert_with(|| (0, resolve(&set, &map_handle(m), s.material)));
                    entry.0 += 1;
                }
                println!(
                    "bsp nodes={} surfs={} points={} zones={} drawn_polygons={drawn} triangles={tris} skipped={skipped:?}",
                    model.nodes.len(),
                    model.surfs.len(),
                    model.points.len(),
                    model.num_zones
                );
                println!("bsp drawn bounds min={lo:?} max={hi:?}");
            }
            Err(e) => {
                bsp_ok = false;
                println!("bsp FAILED: {e}");
            }
        }
    }

    // Static mesh actors
    let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unique: HashMap<String, Option<ObjectHandle>> = HashMap::new();
    let (mut pitch_or_roll, mut pre_pivot, mut nonuniform) = (0usize, 0usize, 0usize);
    for a in &contents.mesh_actors {
        *by_class.entry(a.class.as_str()).or_default() += 1;
        if a.rotation.pitch != 0 || a.rotation.roll != 0 {
            pitch_or_roll += 1;
        }
        if a.pre_pivot != [0.0; 3] {
            pre_pivot += 1;
        }
        if a.scale[0] != a.scale[1] || a.scale[1] != a.scale[2] {
            nonuniform += 1;
        }
        let key = pkg.object_path(a.mesh);
        unique
            .entry(key)
            .or_insert_with(|| set.resolve(&lp, a.mesh).filter(|h| h.class_name() == "StaticMesh"));
    }
    println!("mesh actors by class: {by_class:?}");
    println!(
        "mesh actors with pitch_or_roll={pitch_or_roll} pre_pivot={pre_pivot} nonuniform_scale={nonuniform}"
    );
    println!("skipped mesh actors: {:?}", contents.skipped);
    let with_skins = contents.mesh_actors.iter().filter(|a| a.skins.iter().any(|s| *s != ObjectRef::Null)).count();
    println!("mesh actors with Skins overrides={with_skins}");
    for a in &contents.mesh_actors {
        if pkg.object_path(a.mesh).ends_with("BasicCube") {
            let skins: Vec<String> = a.skins.iter().map(|s| pkg.object_path(*s)).collect();
            println!("BasicCube actor {} skins={skins:?}", pkg.object_path(ObjectRef::Export(a.export)));
        }
    }

    let (mut mesh_ok, mut mesh_unresolved, mut mesh_failed) = (0usize, 0usize, 0usize);
    let (mut mesh_tris, mut mesh_verts) = (0usize, 0usize);
    let mut unresolved_examples = Vec::new();
    for (path, h) in &unique {
        let Some(h) = h else {
            mesh_unresolved += 1;
            if unresolved_examples.len() < 5 {
                unresolved_examples.push(path.clone());
            }
            continue;
        };
        match ue_assets::static_mesh::read_static_mesh(&h.package.pkg, h.export) {
            Ok(m) => {
                mesh_ok += 1;
                mesh_verts += m.positions.len();
                mesh_tris += m.sections.iter().map(|s| s.num_triangles).sum::<usize>();
                for rf in &m.materials {
                    let key = format!("{}:{:?}", h.path(), rf);
                    let entry = materials.entry(key).or_insert_with(|| (0, resolve(&set, h, *rf)));
                    entry.0 += 1;
                }
            }
            Err(e) => {
                mesh_failed += 1;
                println!("mesh FAILED {}: {e}", h.path());
            }
        }
    }
    println!(
        "unique meshes={} decoded={mesh_ok} unresolved={mesh_unresolved} failed={mesh_failed} vertices={mesh_verts} triangles={mesh_tris}",
        unique.len()
    );
    if !unresolved_examples.is_empty() {
        println!("unresolved mesh examples: {unresolved_examples:?}");
    }

    // Materials
    let mut blends: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_texture: BTreeMap<String, usize> = BTreeMap::new();
    let mut textures: HashMap<String, ObjectHandle> = HashMap::new();
    fn key_of(
        materials: &HashMap<String, (usize, ue_assets::material::SimpleMaterial)>,
        m: &ue_assets::material::SimpleMaterial,
    ) -> String {
        materials
            .iter()
            .find(|(_, (_, x))| std::ptr::eq(x, m))
            .map_or(String::new(), |(k, (n, _))| format!("{k} (used {n}x)"))
    }
    for (_, m) in materials.values() {
        *blends.entry(format!("{:?}", m.blend)).or_default() += 1;
        match &m.texture {
            Some(t) => {
                textures.insert(t.path(), t.clone());
            }
            None if m.blend != Blend::Invisible => {
                *no_texture.entry(m.chain.join(">")).or_default() += 1;
                println!("material without texture: {} chain={:?}", key_of(&materials, m), m.chain);
            }
            None => {}
        }
    }
    let mut tex_ok = 0usize;
    let mut tex_bad: BTreeMap<String, usize> = BTreeMap::new();
    for t in textures.values() {
        match read_texture(&t.package.pkg, t.export) {
            Ok(tex) => {
                let palette = match tex.palette_ref {
                    ObjectRef::Null => None,
                    rf => set
                        .resolve(&t.package, rf)
                        .and_then(|h| read_palette(&h.package.pkg, h.export).ok()),
                };
                if tex.mips.first().is_some_and(|m| decode_rgba(tex.format, m, palette.as_deref()).is_some()) {
                    tex_ok += 1;
                } else {
                    *tex_bad.entry(format!("{:?}", tex.format)).or_default() += 1;
                }
            }
            Err(_) => *tex_bad.entry("read_error".into()).or_default() += 1,
        }
    }
    println!("materials={} blends={blends:?}", materials.len());
    println!("materials without texture (by chain): {no_texture:?}");
    println!("unique textures={} decodable={tex_ok} not_decodable={tex_bad:?}", textures.len());
    println!(
        "packages loaded={} missing={:?} seconds={:.1}",
        set.loaded_count(),
        set.missing.borrow(),
        started.elapsed().as_secs_f64()
    );
    if let Some(ps) = contents.player_starts.first() {
        println!("first player start location={:?} rotation={:?}", ps.location, ps.rotation);
    }
    Ok(bsp_ok && mesh_failed == 0)
}

fn resolve_path(install: &Install, file: &str) -> PathBuf {
    resolve(install, file)
}

/// One line per BSP surface material: how many polygons, whether and how they
/// are drawn, texture alpha, and where they are. Written to
/// `logs/kfpkg-bspmaterials-<map>.log`, sorted by polygon count.
fn bsp_materials(install: &Install, map: &str) -> Result<bool, String> {
    use std::collections::BTreeMap;
    use ue_assets::bsp::{poly_flags, read_model};
    use ue_assets::material::resolve;
    use ue_assets::package_set::{ObjectHandle, PackageSet};

    let set = PackageSet::new(&install.root);
    let lp = set
        .load_path(&install.root.join("Maps").join(format!("{map}.rom")))
        .map_err(|e| e.to_string())?;
    let contents = ue_assets::level::read_level(&lp.pkg);
    let m = contents.bsp_model.ok_or("no BSP model")?;
    let model = read_model(&lp.pkg, m).map_err(|e| e.to_string())?;
    let handle = ObjectHandle { package: lp.clone(), export: m };

    #[derive(Default)]
    struct Group {
        polys: usize,
        flags: BTreeMap<u32, usize>,
        lo: [f32; 3],
        hi: [f32; 3],
    }
    let mut groups: BTreeMap<String, (ObjectRef, Group)> = BTreeMap::new();
    for (i, n) in model.nodes.iter().enumerate() {
        let s = &model.surfs[n.surf];
        let key = lp.pkg.object_path(s.material);
        let g = &mut groups
            .entry(key)
            .or_insert_with(|| (s.material, Group { lo: [f32::MAX; 3], hi: [f32::MIN; 3], ..Default::default() }))
            .1;
        g.polys += 1;
        *g.flags.entry(s.flags).or_default() += 1;
        for p in model.node_polygon(i) {
            for (a, v) in p.iter().enumerate() {
                g.lo[a] = g.lo[a].min(*v);
                g.hi[a] = g.hi[a].max(*v);
            }
        }
    }
    let mut rows: Vec<_> = groups.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.1.polys));
    let log_path = format!("logs/kfpkg-bspmaterials-{map}.log");
    let mut log = create_log(&log_path)?;
    for (path, (rf, g)) in &rows {
        let mat = resolve(&set, &handle, *rf);
        let (fmt, mean_alpha, low_alpha) = match &mat.texture {
            Some(t) => match read_texture(&t.package.pkg, t.export) {
                Ok(tex) => {
                    let pal = match tex.palette_ref {
                        ObjectRef::Null => None,
                        prf => set.resolve(&t.package, prf).and_then(|h| read_palette(&h.package.pkg, h.export).ok()),
                    };
                    match tex.mips.first().and_then(|m0| decode_rgba(tex.format, m0, pal.as_deref())) {
                        Some(px) => {
                            let n = (px.len() / 4).max(1);
                            let sum: usize = px.iter().skip(3).step_by(4).map(|&a| a as usize).sum();
                            let low = px.iter().skip(3).step_by(4).filter(|&&a| a < 128).count();
                            (format!("{:?}", tex.format), sum / n, format!("{:.0}%", 100.0 * low as f64 / n as f64))
                        }
                        None => (format!("{:?}", tex.format), 0, "?".into()),
                    }
                }
                Err(_) => ("read_error".into(), 0, "?".into()),
            },
            None => ("none".into(), 0, "-".into()),
        };
        let skipped: usize = g
            .flags
            .iter()
            .filter(|(f, _)| *f & (poly_flags::INVISIBLE | poly_flags::PORTAL | poly_flags::FAKE_BACKDROP) != 0)
            .map(|(_, n)| n)
            .sum();
        let flags: Vec<String> = g.flags.iter().map(|(f, n)| format!("{f:#x}x{n}")).collect();
        let line = format!(
            "polys={:<5} skipped_by_flags={:<4} blend={:?} two_sided={} chain={} texture_format={fmt} mean_alpha={mean_alpha} alpha_below_half={low_alpha} flags=[{}] bounds=({:.0},{:.0},{:.0})..({:.0},{:.0},{:.0}) material={path} texture={}",
            g.polys,
            skipped,
            mat.blend,
            mat.two_sided,
            mat.chain.join(">"),
            flags.join(" "),
            g.lo[0], g.lo[1], g.lo[2], g.hi[0], g.hi[1], g.hi[2],
            mat.texture.as_ref().map_or("none".to_string(), |t| t.path()),
        );
        let _ = writeln!(log, "{line}");
    }
    println!("materials={} written={log_path}", rows.len());
    Ok(true)
}

/// Per BSP zone: the ZoneInfo actor (class, location), how many polygons face
/// into it, how many of those are fake-backdrop (sky window) polygons, and
/// their bounds. Also lists SkyZoneInfo actors.
fn zones(install: &Install, map: &str) -> Result<bool, String> {
    use ue_assets::bsp::{poly_flags, read_model};
    let path = install.root.join("Maps").join(format!("{map}.rom"));
    let p = Package::open(&path).map_err(|e| e.to_string())?;
    let contents = ue_assets::level::read_level(&p);
    let model = read_model(&p, contents.bsp_model.ok_or("no BSP")?).map_err(|e| e.to_string())?;
    let location = |rf: ObjectRef| match rf {
        ObjectRef::Export(i) => read_export_properties(&p, i).ok().and_then(|pl| match pl.get(&p, "Location") {
            Some(ue_assets::properties::Value::Vector(v)) => Some(*v),
            _ => None,
        }),
        _ => None,
    };
    for (z, actor) in model.zone_actors.iter().enumerate() {
        let (mut polys, mut backdrop) = (0usize, 0usize);
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        let (mut blo, mut bhi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (i, n) in model.nodes.iter().enumerate() {
            // zone[1] is the zone on the front side of the node, where a viewer sees it from.
            if n.zone[1] as usize != z {
                continue;
            }
            polys += 1;
            let is_backdrop = model.surfs[n.surf].flags & poly_flags::FAKE_BACKDROP != 0;
            if is_backdrop {
                backdrop += 1;
            }
            for q in model.node_polygon(i) {
                for a in 0..3 {
                    lo[a] = lo[a].min(q[a]);
                    hi[a] = hi[a].max(q[a]);
                    if is_backdrop {
                        blo[a] = blo[a].min(q[a]);
                        bhi[a] = bhi[a].max(q[a]);
                    }
                }
            }
        }
        let class = match actor {
            ObjectRef::Export(i) => p.export_class_name(*i).to_string(),
            _ => "none".into(),
        };
        println!(
            "zone={z} actor={} class={class} actor_location={:?} polygons={polys} backdrop_polygons={backdrop} bounds={lo:?}..{hi:?} backdrop_bounds={blo:?}..{bhi:?}",
            p.object_path(*actor),
            location(*actor),
        );
    }
    for i in 0..p.exports.len() {
        let c = p.export_class_name(i);
        if c.contains("SkyZone") {
            println!("sky actor {} class={c} location={:?}", p.object_path(ObjectRef::Export(i)), location(ObjectRef::Export(i)));
        }
    }
    Ok(true)
}

/// Every BSP polygon with zone `z` on either side: surface material, flags,
/// normal, centre, size, and the zones on each side.
fn zone_polygons(install: &Install, map: &str, z: u8) -> Result<bool, String> {
    use ue_assets::bsp::read_model;
    let path = install.root.join("Maps").join(format!("{map}.rom"));
    let p = Package::open(&path).map_err(|e| e.to_string())?;
    let contents = ue_assets::level::read_level(&p);
    let model = read_model(&p, contents.bsp_model.ok_or("no BSP")?).map_err(|e| e.to_string())?;
    for (i, n) in model.nodes.iter().enumerate() {
        if n.zone[0] != z && n.zone[1] != z {
            continue;
        }
        let s = &model.surfs[n.surf];
        let pts: Vec<[f32; 3]> = model.node_polygon(i).collect();
        let mut c = [0f32; 3];
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for q in &pts {
            for a in 0..3 {
                c[a] += q[a] / pts.len() as f32;
                lo[a] = lo[a].min(q[a]);
                hi[a] = hi[a].max(q[a]);
            }
        }
        let nrm = model.vectors[s.normal];
        println!(
            "node={i} zones(back,front)={:?} flags={:#x} normal=({:.2},{:.2},{:.2}) centre=({:.0},{:.0},{:.0}) size=({:.0},{:.0},{:.0}) verts={} material={}",
            n.zone, s.flags, nrm[0], nrm[1], nrm[2], c[0], c[1], c[2],
            hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2], n.num_verts,
            p.object_path(s.material)
        );
    }
    Ok(true)
}

/// Reads every TerrainInfo in a map and checks the height formula: for each
/// PathNode/PlayerStart over the terrain, (actor Z - terrain height) should be
/// nearly the same everywhere if the formula and axis orientation are right.
fn terrain(install: &Install, map: &str, at: Option<&str>) -> Result<bool, String> {
    use ue_assets::package_set::PackageSet;
    use ue_assets::properties::Value;
    let set = PackageSet::new(&install.root);
    let lp = set
        .load_path(&install.root.join("Maps").join(format!("{map}.rom")))
        .map_err(|e| e.to_string())?;
    let pkg = &lp.pkg;
    let terrains = ue_assets::terrain::read_terrains(&set, &lp);
    // Ground-placed navigation actors.
    let mut ground: Vec<(String, [f32; 3])> = Vec::new();
    for i in 0..pkg.exports.len() {
        let c = pkg.export_class_name(i);
        if (c == "PathNode" || c.contains("PlayerStart"))
            && let Ok(p) = read_export_properties(pkg, i)
            && let Some(Value::Vector(v)) = p.get(pkg, "Location")
        {
            ground.push((c.to_string(), *v));
        }
    }
    let mut ok = true;
    for (ti, t) in terrains.iter().enumerate() {
        let t = match t {
            Ok(t) => t,
            Err(e) => {
                ok = false;
                println!("terrain {ti}: FAILED {e}");
                continue;
            }
        };
        let (mut zmin, mut zmax) = (f32::MAX, f32::MIN);
        for y in 0..t.height {
            for x in 0..t.width {
                let z = t.vertex(x, y)[2];
                zmin = zmin.min(z);
                zmax = zmax.max(z);
            }
        }
        println!(
            "terrain {ti}: {} heightmap={}x{} location={:?} scale={:?} z_range={zmin:.0}..{zmax:.0} layers={} visible_quads={:.1}% inverted={} zone={:?}",
            pkg.object_path(ObjectRef::Export(t.export)),
            t.width, t.height, t.location, t.scale, t.layers.len(),
            100.0 * t.visible_fraction(), t.inverted, t.zone_number
        );
        for (li, l) in t.layers.iter().enumerate() {
            println!(
                "  layer {li}: texture={} alpha_map={} uv_per_unit=({:.5}, {:.5})",
                pkg.object_path(l.texture), pkg.object_path(l.alpha_map), l.matrix[0][0], l.matrix[1][1]
            );
        }
        if let Some(at) = at {
            let v: Vec<f32> = at.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() >= 2 {
                println!(
                    "  at ({}, {}): quad={:?} visible={:?} ground_z={:?}",
                    v[0], v[1],
                    t.quad_at(v[0], v[1]),
                    t.quad_at(v[0], v[1]).map(|(x, y)| t.quad_visible(x, y)),
                    t.height_at(v[0], v[1])
                );
            }
        }
        let in_grid: Vec<(usize, usize)> = ground.iter().filter_map(|(_, p)| t.quad_at(p[0], p[1])).collect();
        let over_visible = in_grid.iter().filter(|(x, y)| t.quad_visible(*x, *y)).count();
        println!(
            "  hole check: ground actors inside grid={} over visible quads={} over holes={}",
            in_grid.len(),
            over_visible,
            in_grid.len() - over_visible
        );
        let mut offsets: Vec<f32> = ground
            .iter()
            .filter_map(|(_, p)| t.height_at(p[0], p[1]).map(|h| p[2] - h))
            .collect();
        offsets.sort_by(|a, b| a.total_cmp(b));
        if offsets.is_empty() {
            println!("  height check: no ground actors over this terrain");
            continue;
        }
        let pct = |q: f32| offsets[((offsets.len() - 1) as f32 * q) as usize];
        let near = offsets.iter().filter(|o| (**o - pct(0.5)).abs() < 16.0).count();
        println!(
            "  height check: actors_over_terrain={} offset(actor Z - ground) p10={:.1} median={:.1} p90={:.1} within_16_of_median={}",
            offsets.len(), pct(0.1), pct(0.5), pct(0.9), near
        );
    }
    Ok(ok)
}

/// Names of every property object across all script packages (lowercase).
fn all_property_names(install: &Install) -> Result<std::collections::HashSet<String>, String> {
    let mut names = std::collections::HashSet::new();
    for (dir, path) in package_files(install)? {
        if dir != "System" {
            continue;
        }
        let p = Package::open(&path).map_err(|e| e.to_string())?;
        ue_assets::properties::add_class_property_names(&p, &mut names);
    }
    Ok(names)
}

/// Prints a class's default properties, e.g. `kfpkg defaults Engine.Pawn`.
fn class_defaults(install: &Install, class: &str) -> Result<bool, String> {
    if class == "--all" {
        return all_class_defaults(install);
    }
    let (pkg_name, class_name) = class.split_once('.').ok_or("use Package.Class")?;
    let names = all_property_names(install)?;
    let p = open(install, &format!("System/{pkg_name}.u"))?;
    let i = (0..p.exports.len())
        .find(|&i| p.export_class_name(i) == "Class" && p.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name))
        .ok_or_else(|| format!("no class {class_name} in {pkg_name}"))?;
    let super_name = p.object_path(p.exports[i].super_ref);
    match ue_assets::properties::find_class_defaults(&p, i, &names) {
        Some((list, start)) => {
            println!(
                "class {class} extends {super_name}: {} defaults, starting at byte {start} of {}",
                list.props.len(),
                p.exports[i].serial_size
            );
            for prop in &list.props {
                let idx = if prop.array_index > 0 { format!("[{}]", prop.array_index) } else { String::new() };
                println!("    {}{idx} = {}", p.name(prop.name), DisplayValue(&p, &prop.value));
            }
            Ok(true)
        }
        None => {
            println!("class {class}: defaults not found");
            Ok(false)
        }
    }
}

/// One line per script class: where its defaults start and how many there
/// are (for comparing changes to the defaults finder).
fn all_class_defaults(install: &Install) -> Result<bool, String> {
    let names = all_property_names(install)?;
    let (mut found, mut missing) = (0, 0);
    for (dir, path) in package_files(install)? {
        if dir != "System" {
            continue;
        }
        let p = Package::open(&path).map_err(|e| e.to_string())?;
        for i in 0..p.exports.len() {
            if p.export_class_name(i) != "Class" {
                continue;
            }
            match ue_assets::properties::find_class_defaults(&p, i, &names) {
                Some((list, start)) => {
                    found += 1;
                    println!("{} start={start} count={}", p.object_path(ObjectRef::Export(i)), list.props.len());
                }
                None => {
                    missing += 1;
                    println!("{} not_found", p.object_path(ObjectRef::Export(i)));
                }
            }
        }
    }
    eprintln!("classes with defaults: {found}, not found: {missing}");
    Ok(true)
}

/// Reads every Polys export and every brush Model's Polys reference.
fn scan_polys(install: &Install) -> Result<bool, String> {
    let (mut total, mut failed, mut polygons) = (0usize, 0usize, 0usize);
    let (mut models, mut models_failed) = (0usize, 0usize);
    let mut first_error = None;
    for (dir, path) in package_files(install)? {
        if dir != "Maps" {
            continue;
        }
        let p = Package::open(&path).map_err(|e| e.to_string())?;
        for i in 0..p.exports.len() {
            match p.export_class_name(i) {
                "Polys" => {
                    total += 1;
                    match ue_assets::bsp::read_polys(&p, i) {
                        Ok(v) => polygons += v.len(),
                        Err(e) => {
                            failed += 1;
                            first_error.get_or_insert_with(|| format!("{} export {i}: {e}", path.display()));
                        }
                    }
                }
                "Model" => {
                    models += 1;
                    match ue_assets::bsp::read_model(&p, i) {
                        Ok(m) if matches!(m.polys, ObjectRef::Null) || matches!(m.polys, ObjectRef::Export(e) if p.export_class_name(e) == "Polys") => {}
                        Ok(m) => {
                            models_failed += 1;
                            first_error.get_or_insert_with(|| format!("model {i}: polys ref {:?} is not a Polys", m.polys));
                        }
                        Err(e) => {
                            models_failed += 1;
                            first_error.get_or_insert_with(|| format!("{} model {i}: {e}", path.display()));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    println!("summary polys_objects={total} failed={failed} polygons={polygons} models={models} models_failed={models_failed}");
    if let Some(e) = first_error {
        println!("first error: {e}");
    }
    Ok(failed == 0 && models_failed == 0)
}

/// Bounds of a brush actor's polygons: in brush space, and in world space with
/// the pivot subtracted or added, to settle how PrePivot applies.
fn brush_bounds(install: &Install, map: &str, actor: &str) -> Result<bool, String> {
    use ue_assets::properties::Value;
    let p = open(install, &format!("Maps/{map}.rom"))?;
    let i = (0..p.exports.len())
        .find(|&i| p.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(actor))
        .ok_or("actor not found")?;
    let props = read_export_properties(&p, i).map_err(|e| e.to_string())?;
    let v = |n: &str| match props.get(&p, n) {
        Some(Value::Vector(v)) => *v,
        _ => [0.0; 3],
    };
    let (loc, pivot) = (v("Location"), v("PrePivot"));
    let model = match props.get(&p, "Brush") {
        Some(Value::Object(ObjectRef::Export(m))) => *m,
        _ => return Err("no brush".into()),
    };
    let m = ue_assets::bsp::read_model(&p, model).map_err(|e| e.to_string())?;
    let ObjectRef::Export(pe) = m.polys else { return Err("no polys".into()) };
    let polys = ue_assets::bsp::read_polys(&p, pe).map_err(|e| e.to_string())?;
    let bounds = |f: &dyn Fn([f32; 3]) -> [f32; 3]| {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for q in polys.iter().flat_map(|q| q.vertices.iter()) {
            let w = f(*q);
            for a in 0..3 {
                lo[a] = lo[a].min(w[a]);
                hi[a] = hi[a].max(w[a]);
            }
        }
        (lo, hi)
    };
    println!("{actor}: Location={loc:?} PrePivot={pivot:?} polygons={}", polys.len());
    println!("  local bounds {:?}", bounds(&|q| q));
    println!("  world (v - PrePivot + Location) {:?}", bounds(&|q| [q[0] - pivot[0] + loc[0], q[1] - pivot[1] + loc[1], q[2] - pivot[2] + loc[2]]));
    println!("  world (v + Location)            {:?}", bounds(&|q| [q[0] + loc[0], q[1] + loc[1], q[2] + loc[2]]));
    Ok(true)
}

/// Reads every SkeletalMesh; checks weights and that bone names match the
/// mesh's own animation set when it is in the same package.
/// Parses every Karma ragdoll file (`KarmaData/*.ka`) and lists its ragdolls.
fn scan_karma(install: &Install) -> Result<bool, String> {
    use ue_assets::karma::{JointKind, parse_ka};
    let dir = install.root.join("KarmaData");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ka")))
        .collect();
    files.sort();
    let mut ok = true;
    for path in files {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&bytes);
        match parse_ka(&text) {
            Ok(all) => {
                println!("{}: {} ragdolls", path.file_name().unwrap().to_string_lossy(), all.len());
                let mut names: Vec<_> = all.keys().collect();
                names.sort();
                for n in names {
                    let r = &all[n];
                    let hinges = r.joints.iter().filter(|j| matches!(j.kind, JointKind::Hinge { .. })).count();
                    let shapes: usize = r.parts.iter().map(|p| p.primitives.len()).sum();
                    let root: Vec<&str> = r.parts.iter().filter(|p| p.parent.is_none()).map(|p| p.bone.as_str()).collect();
                    println!(
                        "  {n}: scale={} parts={} shapes={shapes} joints={} (hinges={hinges}) no_collision_pairs={} root={root:?}",
                        r.scale,
                        r.parts.len(),
                        r.joints.len(),
                        r.no_collision.len()
                    );
                }
            }
            Err(e) => {
                ok = false;
                println!("{}: error: {e}", path.display());
            }
        }
    }
    Ok(ok)
}

/// Prints a map's navigation network: nodes by class, edges by flags,
/// connected groups. `map` is a name like KF-WestLondon, or `all`.
fn nav(install: &Install, map: &str) -> Result<bool, String> {
    use ue_assets::nav::*;
    let maps: Vec<PathBuf> = if map == "all" {
        let mut v: Vec<PathBuf> = std::fs::read_dir(install.root.join("Maps"))
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rom")))
            .collect();
        v.sort();
        v
    } else {
        vec![install.root.join("Maps").join(format!("{map}.rom"))]
    };
    for path in maps {
        let p = Package::open(&path).map_err(|e| e.to_string())?;
        let g = read_nav(&p);
        let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
        for n in &g.nodes {
            *classes.entry(n.class.as_str()).or_default() += 1;
        }
        let names = [
            (R_WALK, "walk"),
            (R_FLY, "fly"),
            (R_SWIM, "swim"),
            (R_JUMP, "jump"),
            (R_DOOR, "door"),
            (R_SPECIAL, "special"),
            (R_LADDER, "ladder"),
            (R_PROSCRIBED, "proscribed"),
            (R_FORCED, "forced"),
            (R_PLAYERONLY, "playeronly"),
        ];
        let mut flags: BTreeMap<&str, usize> = BTreeMap::new();
        for e in &g.edges {
            for (bit, name) in names {
                if e.flags & bit != 0 {
                    *flags.entry(name).or_default() += 1;
                }
            }
        }
        // Edges a 24 x 44 zed may walk (see DESIGN.md, Pathfinding).
        let usable = g
            .edges
            .iter()
            .filter(|e| e.flags & !(R_WALK | R_FORCED | R_DOOR) == 0 && e.radius >= 24.0 && e.height >= 44.0)
            .count();
        let (sizes, _) = g.groups();
        println!(
            "{} nodes={} edges={} broken={} usable_by_zeds={usable} groups={} largest={:?} classes={classes:?} flags={flags:?}",
            path.file_stem().map_or(String::new(), |s| s.to_string_lossy().to_string()),
            g.nodes.len(),
            g.edges.len(),
            g.broken_specs,
            sizes.len(),
            &sizes[..sizes.len().min(5)]
        );
    }
    Ok(true)
}

/// Prints a particle effect (Emitter class) with every sub-emitter's values.
fn emitter(install: &Install, class: &str) -> Result<bool, String> {
    let set = ue_assets::package_set::PackageSet::new(&install.root);
    let defaults = ue_assets::class_defaults::ClassDefaults::new(&set);
    let e = ue_assets::emitter::read_emitter_class(&set, &defaults, class)?;
    println!("{} auto_destroy={} life_span={} emitters={}", e.class, e.auto_destroy, e.life_span, e.emitters.len());
    for (i, (d, _)) in e.emitters.iter().enumerate() {
        println!("[{i}] {d:#?}");
    }
    Ok(true)
}

/// Prints a skeletal mesh's bones and candidate attachment tag tables.
/// Prints each sequence's animation notifies (time 0..1, function, notify
/// object class and its NotifyName).
fn notifies(install: &Install, file: &str, anim: &str) -> Result<bool, String> {
    use ue_assets::skeletal::read_mesh_animation;
    let p = Package::open(&install.root.join(file)).map_err(|e| e.to_string())?;
    let i = (0..p.exports.len())
        .find(|&i| p.export_class_name(i) == "MeshAnimation" && p.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(anim))
        .ok_or(format!("no MeshAnimation {anim} in {file}"))?;
    let a = read_mesh_animation(&p, i).map_err(|e| e.to_string())?;
    for s in &a.sequences {
        for n in &s.notifies {
            println!("{} frames={} time={:.3} function={} object={} name={} effect={:?}", s.name, s.num_frames, n.time, n.function, n.object_class, n.name, n.effect);
        }
    }
    Ok(true)
}

fn mesh_tags(install: &Install, file: &str, mesh: &str) -> Result<bool, String> {
    use ue_assets::skeletal::{find_attach_tags, read_skeletal_mesh};
    let p = Package::open(&install.root.join(file)).map_err(|e| e.to_string())?;
    let i = (0..p.exports.len())
        .find(|&i| p.export_class_name(i) == "SkeletalMesh" && p.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(mesh))
        .ok_or("mesh not found")?;
    let m = read_skeletal_mesh(&p, i).map_err(|e| e.to_string())?;
    let names: Vec<&str> = m.bones.iter().map(|b| b.name.as_str()).collect();
    println!("bones ({}): {}", names.len(), names.join(" "));
    for t in &m.tags {
        println!("tag {} -> {} origin={:?} axes={:?}", t.alias, t.bone, t.origin, t.axes);
    }
    let data = p.export_data(i);
    for (at, end, tags) in find_attach_tags(&p, i, &m.bones) {
        if tags.len() >= 3 {
            println!("tag table candidate at byte {at}..{end} of {}: {tags:?}", data.len());
            // What follows, as raw bytes and as floats, to look for tag coordinates.
            let after = &data[end..(end + 16 * 52).min(data.len())];
            println!("  next bytes: {:02x?}", &after[..after.len().min(16)]);
            let floats: Vec<String> = after[1..]
                .as_chunks::<4>()
                .0
                .iter()
                .take(12 * 3)
                .map(|c| format!("{:.3}", f32::from_le_bytes(*c)))
                .collect();
            println!("  floats from byte {}: {}", end + 1, floats.join(" "));
        }
    }
    Ok(true)
}

fn scan_skeletal(install: &Install) -> Result<bool, String> {
    use ue_assets::skeletal::{read_mesh_animation, read_skeletal_mesh};
    let log_path = "logs/kfpkg-skelmeshes.log";
    let mut log = create_log(log_path)?;
    let (mut total, mut failed, mut bad_weights, mut anim_checked, mut anim_mismatch) = (0usize, 0, 0, 0, 0);
    let mut first_errors = Vec::new();
    for (_, path) in package_files(install)? {
        let rel = path.strip_prefix(&install.root).unwrap_or(&path).display().to_string();
        let p = Package::open(&path).map_err(|e| e.to_string())?;
        for i in 0..p.exports.len() {
            if p.export_class_name(i) != "SkeletalMesh" {
                continue;
            }
            total += 1;
            let name = p.object_path(ObjectRef::Export(i));
            match read_skeletal_mesh(&p, i) {
                Ok(m) => {
                    let sums = m.weight_sums();
                    let off = sums.iter().filter(|s| (**s - 1.0).abs() > 0.02).count();
                    if off > 0 {
                        bad_weights += 1;
                    }
                    let mut anim_note = String::from("anim=other_package");
                    if let ObjectRef::Export(a) = m.animation {
                        anim_checked += 1;
                        match read_mesh_animation(&p, a) {
                            Ok(anim) => {
                                let mesh_names: std::collections::HashSet<&str> = m.bones.iter().map(|b| b.name.as_str()).collect();
                                let missing = anim.bones.iter().filter(|b| !mesh_names.contains(b.as_str())).count();
                                if missing > 0 {
                                    anim_mismatch += 1;
                                }
                                anim_note = format!("anim_bones={} anim_bones_not_in_mesh={missing} sequences={}", anim.bones.len(), anim.sequences.len());
                            }
                            Err(e) => {
                                anim_mismatch += 1;
                                anim_note = format!("anim_error=\"{e}\"");
                            }
                        }
                    }
                    let _ = writeln!(
                        log,
                        "ok file=\"{rel}\" mesh={name} bones={} points={} wedges={} triangles={} influences={} materials={} points_with_bad_weight_sum={off} scale={:?} rot_origin={:?} {anim_note}",
                        m.bones.len(), m.points.len(), m.wedges.len(), m.triangles.len(), m.influences.len(), m.material_slots.len(), m.scale, m.rot_origin
                    );
                }
                Err(e) => {
                    failed += 1;
                    let msg = format!("fail file=\"{rel}\" mesh={name} error=\"{e}\"");
                    let _ = writeln!(log, "{msg}");
                    if first_errors.len() < 8 {
                        first_errors.push(msg);
                    }
                }
            }
        }
    }
    for e in &first_errors {
        println!("{e}");
    }
    let summary = format!(
        "summary skeletal_meshes={total} ok={} failed={failed} meshes_with_bad_weights={bad_weights} same_package_anims_checked={anim_checked} anim_mismatch={anim_mismatch}",
        total - failed
    );
    println!("{summary}");
    let _ = writeln!(log, "{summary}");
    println!("details: {log_path}");
    Ok(failed == 0)
}

/// Reads every MeshAnimation; checks key counts and rotation lengths.
fn scan_anims(install: &Install) -> Result<bool, String> {
    use ue_assets::skeletal::read_mesh_animation;
    let (mut total, mut failed, mut sequences, mut tracks, mut bad_keys, mut non_unit) = (0usize, 0, 0, 0, 0, 0);
    let mut first_errors = Vec::new();
    for (_, path) in package_files(install)? {
        let rel = path.strip_prefix(&install.root).unwrap_or(&path).display().to_string();
        let p = Package::open(&path).map_err(|e| e.to_string())?;
        for i in 0..p.exports.len() {
            if p.export_class_name(i) != "MeshAnimation" {
                continue;
            }
            total += 1;
            match read_mesh_animation(&p, i) {
                Ok(a) => {
                    for s in &a.sequences {
                        sequences += 1;
                        for t in &s.tracks {
                            tracks += 1;
                            let n = t.times.len();
                            // Each key list has one key per time, or a single constant key.
                            let ok = |k: usize| k == n || k == 1;
                            if !ok(t.rotations.len()) || !ok(t.positions.len()) || t.times.iter().any(|&x| x < 0.0 || x > s.track_time + 0.01) {
                                bad_keys += 1;
                            }
                            if t.rotations.iter().any(|q| (q.iter().map(|c| c * c).sum::<f32>() - 1.0).abs() > 0.01) {
                                non_unit += 1;
                            }
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    if first_errors.len() < 8 {
                        first_errors.push(format!("{rel} {}: {e}", p.object_path(ObjectRef::Export(i))));
                    }
                }
            }
        }
    }
    for e in &first_errors {
        println!("FAIL {e}");
    }
    println!(
        "summary mesh_animations={total} ok={} failed={failed} sequences={sequences} tracks={tracks} tracks_with_bad_keys={bad_keys} tracks_with_non_unit_rotations={non_unit}",
        total - failed
    );
    Ok(failed == 0)
}
