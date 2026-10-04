//! Karma ragdoll definitions (`KarmaData/*.ka`).
//!
//! These files are plain XML. Each `<ASSET>` is one ragdoll: physics bodies
//! ("parts") tied to skeleton bones, their collision shapes ("primitives"),
//! and the joints between them. Lengths are in file units; multiply by
//! `1 / scale` to get mesh units. All vectors are in the owning bone's frame.

use std::collections::HashMap;

/// One parsed XML element: name, attributes, text, children.
#[derive(Debug, Default)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Element>,
}

impl Element {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    fn float(&self, name: &str) -> Result<f32, String> {
        let c = self.child(name).ok_or_else(|| format!("<{}> has no <{name}>", self.name))?;
        c.text.trim().parse().map_err(|_| format!("<{name}> is not a number: {:?}", c.text))
    }

    fn floats(&self, name: &str) -> Result<Vec<f32>, String> {
        let c = self.child(name).ok_or_else(|| format!("<{}> has no <{name}>", self.name))?;
        c.text
            .split(',')
            .map(|s| s.trim().parse().map_err(|_| format!("<{name}> has a bad number: {:?}", c.text)))
            .collect()
    }

    fn vec3(&self, name: &str) -> Result<[f32; 3], String> {
        let v = self.floats(name)?;
        v.try_into().map_err(|v: Vec<f32>| format!("<{name}> has {} numbers, not 3", v.len()))
    }

    fn matrix(&self) -> Result<Matrix, String> {
        let v = self.floats("TM")?;
        let m: [f32; 16] = v.try_into().map_err(|v: Vec<f32>| format!("<TM> has {} numbers, not 16", v.len()))?;
        Ok(Matrix(m))
    }
}

/// A minimal XML reader: elements, attributes, text. Enough for .ka files
/// (no entities, CDATA or namespaces).
fn parse_xml(src: &str) -> Result<Element, String> {
    let mut stack = vec![Element {
        name: "#document".into(),
        ..Default::default()
    }];
    let mut rest = src;
    while let Some(lt) = rest.find('<') {
        stack.last_mut().unwrap().text.push_str(&rest[..lt]);
        rest = &rest[lt..];
        let gt = rest.find('>').ok_or("unclosed tag")?;
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue; // declaration or comment
        }
        if let Some(name) = tag.strip_prefix('/') {
            let done = stack.pop().ok_or("too many closing tags")?;
            if done.name != name.trim() {
                return Err(format!("</{}> closes <{}>", name.trim(), done.name));
            }
            stack.last_mut().ok_or("closing tag at top level")?.children.push(done);
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, mut attr_src) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        let mut el = Element {
            name: name.to_string(),
            ..Default::default()
        };
        // Attributes: key="value" pairs.
        while let Some(eq) = attr_src.find('=') {
            let key = attr_src[..eq].trim().to_string();
            let after = attr_src[eq + 1..].trim_start();
            let quote = after.chars().next().ok_or("attribute without value")?;
            let end = after[1..].find(quote).ok_or("unclosed attribute")? + 1;
            el.attrs.push((key, after[1..end].to_string()));
            attr_src = &after[end + 1..];
        }
        if self_closing {
            stack.last_mut().unwrap().children.push(el);
        } else {
            stack.push(el);
        }
    }
    if stack.len() != 1 {
        return Err(format!("unclosed <{}>", stack.last().unwrap().name));
    }
    Ok(stack.pop().unwrap())
}

/// A 4x4 transform as stored (row-major; rows 0-2 are the X, Y, Z axes,
/// row 3 is the position), in file units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix(pub [f32; 16]);

impl Matrix {
    pub fn axis(&self, i: usize) -> [f32; 3] {
        [self.0[i * 4], self.0[i * 4 + 1], self.0[i * 4 + 2]]
    }

    pub fn position(&self) -> [f32; 3] {
        self.axis(3)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Sphere { radius: f32 },
    /// Capsule: `height` is the length of the straight part, along the
    /// primitive's local Z axis.
    Sphyl { radius: f32, height: f32 },
    /// Box with full edge lengths along local X, Y, Z.
    Box { dims: [f32; 3] },
}

#[derive(Clone, Debug)]
pub struct Primitive {
    pub shape: Shape,
    /// Placement in the bone's frame.
    pub transform: Matrix,
}

#[derive(Clone, Debug)]
pub enum JointKind {
    /// Ball joint: swing within an elliptical cone around the primary axis,
    /// twist about it within +-`twist`. Angles in radians. `cone_type` and
    /// `twist_type` are the file's CONE_TYPE / TWIST_TYPE (see `TwistType`).
    Skeletal {
        cone_x: f32,
        cone_y: f32,
        twist: f32,
        cone_type: u32,
        twist_type: TwistType,
    },
    /// Rotation about the primary axis only, between `low` and `high` radians
    /// (if `limited`).
    Hinge { low: f32, high: f32, limited: bool },
}

/// TWIST_TYPE of a skeletal joint. The numbering is recalled from
/// MathEngine's Karma API (MdtSkeletalTwistOption: Free, Limited, Locked),
/// not confirmed from KF's files. It fits the data: type-1 joints carry
/// deliberate small angles (spine, neck), type-2 joints mostly the unused
/// default 1.570796.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TwistType {
    Free,
    Limited,
    Locked,
}

impl TwistType {
    fn from_file(v: u32) -> Result<Self, String> {
        match v {
            0 => Ok(TwistType::Free),
            1 => Ok(TwistType::Limited),
            2 => Ok(TwistType::Locked),
            other => Err(format!("unknown TWIST_TYPE {other}")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Joint {
    /// The child part and its parent.
    pub part1: String,
    pub part2: String,
    pub kind: JointKind,
    /// Anchor in each part's frame.
    pub pos1: [f32; 3],
    pub pos2: [f32; 3],
    /// Joint axes in each part's frame.
    pub primary1: [f32; 3],
    pub primary2: [f32; 3],
    pub orthogonal1: [f32; 3],
    pub orthogonal2: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Part {
    /// Bone name.
    pub bone: String,
    pub parent: Option<String>,
    pub mass: f32,
    /// Centre of mass in the bone's frame (MASS_OFFSET).
    pub mass_offset: [f32; 3],
    /// Inertia tensor (xx, xy, xz, yy, yz, zz), file units.
    pub inertia: [f32; 6],
    /// Collision shapes (empty for "dynamics_only" parts).
    pub primitives: Vec<Primitive>,
}

#[derive(Clone, Debug)]
pub struct Ragdoll {
    pub name: String,
    /// File units per mesh unit (0.02 for KF).
    pub scale: f32,
    pub parts: Vec<Part>,
    pub joints: Vec<Joint>,
    /// Part pairs that must not collide with each other.
    pub no_collision: Vec<(String, String)>,
}

/// Reads every ragdoll in a .ka file, by asset name.
pub fn parse_ka(src: &str) -> Result<HashMap<String, Ragdoll>, String> {
    let doc = parse_xml(src)?;
    let karma = doc.child("KARMA").ok_or("no <KARMA> element")?;
    let mut out = HashMap::new();
    for asset in karma.children("ASSET") {
        let r = read_asset(asset).map_err(|e| format!("asset {:?}: {e}", asset.attr("id")))?;
        out.insert(r.name.clone(), r);
    }
    Ok(out)
}

fn read_asset(asset: &Element) -> Result<Ragdoll, String> {
    let name = asset.attr("id").ok_or("asset without id")?.to_string();
    let scale: f32 = asset
        .attr("scale")
        .unwrap_or("1")
        .parse()
        .map_err(|_| "bad scale attribute")?;

    let mut geometries: HashMap<&str, Vec<Primitive>> = HashMap::new();
    for g in asset.children("GEOMETRY") {
        let mut prims = Vec::new();
        for p in g.children("PRIMITIVE") {
            let shape = match p.attr("type") {
                Some("sphere") => Shape::Sphere {
                    radius: p.float("RADIUS")?,
                },
                Some("sphyl") => Shape::Sphyl {
                    radius: p.float("RADIUS")?,
                    height: p.float("HEIGHT")?,
                },
                Some("box") => Shape::Box { dims: p.vec3("DIMS")? },
                other => return Err(format!("unknown primitive type {other:?}")),
            };
            prims.push(Primitive {
                shape,
                transform: p.matrix()?,
            });
        }
        geometries.insert(g.attr("id").ok_or("geometry without id")?, prims);
    }

    // MODEL id -> (geometry id, mass, centre of mass, inertia).
    type Model<'a> = (Option<&'a str>, f32, [f32; 3], [f32; 6]);
    let mut models: HashMap<&str, Model> = HashMap::new();
    for m in asset.children("MODEL") {
        let dynamics = m.child("DYNAMICS");
        let mass = dynamics.map(|d| d.float("MASS")).transpose()?.unwrap_or(1.0);
        let mass_offset = match dynamics.and_then(|d| d.child("MASS_OFFSET")) {
            Some(_) => dynamics.unwrap().vec3("MASS_OFFSET")?,
            None => [0.0; 3],
        };
        let inertia = match dynamics.and_then(|d| d.child("INERTIA")) {
            Some(_) => {
                let v = dynamics.unwrap().floats("INERTIA")?;
                v.try_into().map_err(|v: Vec<f32>| format!("<INERTIA> has {} numbers, not 6", v.len()))?
            }
            None => [0.0; 6],
        };
        let geometry = match m.attr("type") {
            Some("dynamics_only") => None,
            _ => m.attr("geometry"),
        };
        models.insert(m.attr("id").ok_or("model without id")?, (geometry, mass, mass_offset, inertia));
    }

    let mut parts = Vec::new();
    for p in asset.children("PART") {
        let bone = p.attr("id").ok_or("part without id")?;
        let model = p.attr("model").ok_or("part without model")?;
        let (geometry, mass, mass_offset, inertia) = *models.get(model).ok_or_else(|| format!("part {bone}: no model {model}"))?;
        let primitives = match geometry {
            Some(g) => geometries.get(g).cloned().ok_or_else(|| format!("part {bone}: no geometry {g}"))?,
            None => Vec::new(),
        };
        parts.push(Part {
            bone: bone.to_string(),
            parent: p.attr("parent").filter(|s| !s.is_empty()).map(str::to_string),
            mass,
            mass_offset,
            inertia,
            primitives,
        });
    }

    let mut joints = Vec::new();
    for j in asset.children("JOINT") {
        let kind = match j.attr("type") {
            Some("skeletal") => JointKind::Skeletal {
                cone_x: j.float("CONE_HALF_ANGLE_X")?,
                cone_y: j.float("CONE_HALF_ANGLE_Y")?,
                twist: j.float("TWIST_HALF_ANGLE")?,
                cone_type: j.child("CONE_TYPE").map(|_| j.float("CONE_TYPE")).transpose()?.unwrap_or(1.0) as u32,
                twist_type: TwistType::from_file(
                    j.child("TWIST_TYPE").map(|_| j.float("TWIST_TYPE")).transpose()?.unwrap_or(1.0) as u32,
                )?,
            },
            Some("hinge") => JointKind::Hinge {
                low: j.float("LOW_LIMIT")?,
                high: j.float("HIGH_LIMIT")?,
                limited: j.float("LIMITED")? != 0.0,
            },
            other => return Err(format!("unknown joint type {other:?}")),
        };
        joints.push(Joint {
            part1: j.attr("part1").ok_or("joint without part1")?.to_string(),
            part2: j.attr("part2").ok_or("joint without part2")?.to_string(),
            kind,
            pos1: j.vec3("POS1")?,
            pos2: j.vec3("POS2")?,
            primary1: j.vec3("PRIMARY_AXIS1")?,
            primary2: j.vec3("PRIMARY_AXIS2")?,
            orthogonal1: j.vec3("ORTHOGONAL_AXIS1")?,
            orthogonal2: j.vec3("ORTHOGONAL_AXIS2")?,
        });
    }

    let no_collision = asset
        .children("NO_COLLISION")
        .filter_map(|n| Some((n.attr("part1")?.to_string(), n.attr("part2")?.to_string())))
        .collect();

    Ok(Ragdoll {
        name,
        scale,
        parts,
        joints,
        no_collision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<KARMA ka_file_version="1.0">
  <ASSET id="Test" graphic="Test.PSK" scale="0.02" mass_scale="1" length_scale="1">
    <GEOMETRY id="pelvis">
      <PRIMITIVE id="pelvis_1" type="sphere">
        <RADIUS>0.2</RADIUS>
        <TM>1,0,0,0,0,1,0,0,0,0,1,0,0.5,0,0,1</TM>
      </PRIMITIVE>
    </GEOMETRY>
    <GEOMETRY id="thigh">
      <PRIMITIVE id="thigh_1" type="sphyl">
        <RADIUS>0.1</RADIUS>
        <HEIGHT>0.4</HEIGHT>
        <TM>0,0,1,0,0,1,0,0,1,0,0,0,0.25,0,0,1</TM>
      </PRIMITIVE>
    </GEOMETRY>
    <MODEL id="pelvis" type="dynamics_and_geometry" geometry="pelvis">
      <DYNAMICS><MASS>0.1</MASS></DYNAMICS>
    </MODEL>
    <MODEL id="thigh" type="dynamics_and_geometry" geometry="thigh">
      <DYNAMICS><MASS>0.2</MASS><MASS_OFFSET>0.1,0,0</MASS_OFFSET><INERTIA>1,0,0,2,0,3</INERTIA></DYNAMICS>
    </MODEL>
    <MODEL id="collar" type="dynamics_only" geometry="thigh">
      <DYNAMICS><MASS>0.3</MASS></DYNAMICS>
    </MODEL>
    <PART id="pelvis" model="pelvis"><TM>1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1</TM></PART>
    <PART id="thigh" model="thigh" parent="pelvis"><TM>1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1</TM></PART>
    <PART id="collar" model="collar" parent="pelvis"><TM>1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1</TM></PART>
    <JOINT id="thigh" part1="thigh" part2="pelvis" type="skeletal">
      <CONE_HALF_ANGLE_X>0.7</CONE_HALF_ANGLE_X>
      <CONE_HALF_ANGLE_Y>0.3</CONE_HALF_ANGLE_Y>
      <TWIST_TYPE>2</TWIST_TYPE>
      <TWIST_HALF_ANGLE>1.5</TWIST_HALF_ANGLE>
      <POS1>0,0,0</POS1>
      <POS2>0.1,-0.2,0.3</POS2>
      <PRIMARY_AXIS1>1,0,0</PRIMARY_AXIS1>
      <PRIMARY_AXIS2>0,1,0</PRIMARY_AXIS2>
      <ORTHOGONAL_AXIS1>0,1,0</ORTHOGONAL_AXIS1>
      <ORTHOGONAL_AXIS2>1,0,0</ORTHOGONAL_AXIS2>
    </JOINT>
    <JOINT id="collar" part1="collar" part2="pelvis" type="hinge">
      <HIGH_LIMIT>0.2</HIGH_LIMIT>
      <LOW_LIMIT>-2</LOW_LIMIT>
      <LIMITED>1</LIMITED>
      <POS1>0,0,0</POS1>
      <POS2>0,0,0</POS2>
      <PRIMARY_AXIS1>0,0,1</PRIMARY_AXIS1>
      <PRIMARY_AXIS2>0,0,1</PRIMARY_AXIS2>
      <ORTHOGONAL_AXIS1>1,0,0</ORTHOGONAL_AXIS1>
      <ORTHOGONAL_AXIS2>1,0,0</ORTHOGONAL_AXIS2>
    </JOINT>
    <NO_COLLISION part1="thigh" part2="collar"></NO_COLLISION>
  </ASSET>
</KARMA>"#;

    #[test]
    fn reads_sample_ragdoll() {
        let all = parse_ka(SAMPLE).unwrap();
        let r = &all["Test"];
        assert_eq!(r.scale, 0.02);
        assert_eq!(r.parts.len(), 3);
        assert_eq!(r.parts[0].parent, None);
        assert_eq!(r.parts[1].parent.as_deref(), Some("pelvis"));
        assert_eq!(r.parts[1].mass, 0.2);
        assert_eq!(r.parts[1].mass_offset, [0.1, 0.0, 0.0]);
        assert_eq!(r.parts[1].inertia, [1.0, 0.0, 0.0, 2.0, 0.0, 3.0]);
        assert_eq!(
            r.parts[1].primitives[0].shape,
            Shape::Sphyl {
                radius: 0.1,
                height: 0.4
            }
        );
        assert_eq!(r.parts[1].primitives[0].transform.axis(2), [1.0, 0.0, 0.0]);
        assert_eq!(r.parts[1].primitives[0].transform.position(), [0.25, 0.0, 0.0]);
        // dynamics_only parts have no collision shapes.
        assert!(r.parts[2].primitives.is_empty());
        assert_eq!(r.joints.len(), 2);
        assert!(matches!(
            r.joints[0].kind,
            JointKind::Skeletal { cone_x, twist_type: TwistType::Locked, cone_type: 1, .. } if cone_x == 0.7
        ));
        assert!(matches!(r.joints[1].kind, JointKind::Hinge { low, high, limited: true } if low == -2.0 && high == 0.2));
        assert_eq!(r.joints[0].pos2, [0.1, -0.2, 0.3]);
        assert_eq!(r.no_collision, vec![("thigh".to_string(), "collar".to_string())]);
    }

    #[test]
    fn rejects_mismatched_tags() {
        assert!(parse_xml("<A><B></A></B>").is_err());
        assert!(parse_xml("<A>").is_err());
    }
}
