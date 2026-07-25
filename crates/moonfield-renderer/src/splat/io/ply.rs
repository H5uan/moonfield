//! Binary little-endian PLY parser for standard 3DGS scene files.
//!
//! Understands the layout written by the reference 3D Gaussian splatting
//! trainer (and tools like SuperSplat): a single `vertex` element whose float
//! properties include `x, y, z, nx, ny, nz, f_dc_0..2, f_rest_0..44,
//! opacity, scale_0..2, rot_0..3`. Property order is *not* assumed — values
//! are looked up by name, and the unused `nx, ny, nz` normals are skipped.
//!
//! ASCII and big-endian PLY variants are rejected; hand-written parser, no
//! external dependencies.

use std::collections::HashMap;
use std::str::FromStr;

use crate::splat::scene::{GaussianScene, SH_REST_LEN};

/// Errors returned by [`parse_ply`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlyError {
    /// The first line is not the `ply` magic.
    #[error("missing `ply` magic line")]
    MissingMagic,
    /// Only `binary_little_endian 1.0` is supported.
    #[error("unsupported format: {0} (expected `binary_little_endian 1.0`)")]
    UnsupportedFormat(String),
    /// The header has no terminating `end_header` line.
    #[error("missing `end_header` line")]
    MissingEndHeader,
    /// No `element vertex` declaration was found.
    #[error("missing `element vertex` declaration")]
    MissingVertexElement,
    /// A header line could not be understood.
    #[error("invalid header line: {0}")]
    InvalidHeader(String),
    /// The header is not valid UTF-8.
    #[error("invalid UTF-8 in header")]
    InvalidUtf8(#[from] std::str::Utf8Error),
    /// A `property` line uses an unknown scalar type.
    #[error("unsupported property type: {0}")]
    UnsupportedPropertyType(String),
    /// `property list` is not supported by this parser.
    #[error("list properties are not supported: {0}")]
    UnsupportedListProperty(String),
    /// A required vertex property (e.g. `x`, `rot_0`) is absent.
    #[error("missing required vertex property `{0}`")]
    MissingProperty(String),
    /// The body ended before all declared vertices were read.
    #[error("unexpected end of data")]
    UnexpectedEof,
}

/// One parsed `element` block of the header.
struct PlyElement {
    name: String,
    count: usize,
    /// (byte size, property name) in declaration order.
    properties: Vec<(usize, String)>,
}

/// Byte size of a scalar PLY property type name.
fn scalar_size(type_name: &str) -> Result<usize, PlyError> {
    match type_name {
        "char" | "uchar" | "int8" | "uint8" => Ok(1),
        "short" | "ushort" | "int16" | "uint16" => Ok(2),
        "int" | "uint" | "float" | "int32" | "uint32" | "float32" => Ok(4),
        "double" | "float64" => Ok(8),
        other => Err(PlyError::UnsupportedPropertyType(other.to_string())),
    }
}

/// Split the input into the header (up to and including `end_header`) and the
/// body, returning `(header_text, body)`.
fn split_header(bytes: &[u8]) -> Result<(&str, &[u8]), PlyError> {
    // Find the first line from `from` onward; returns (line, next_offset).
    fn next_line(bytes: &[u8], from: usize) -> Option<(&[u8], usize)> {
        let rel_end = bytes[from..].iter().position(|&b| b == b'\n')?;
        let end = from + rel_end;
        let mut line = &bytes[from..end];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        Some((line, end + 1))
    }

    let (magic, mut offset) = next_line(bytes, 0).ok_or(PlyError::MissingEndHeader)?;
    if magic != b"ply" {
        return Err(PlyError::MissingMagic);
    }
    while let Some((line, next)) = next_line(bytes, offset) {
        offset = next;
        if line == b"end_header" {
            let header = std::str::from_utf8(&bytes[..offset])?;
            return Ok((header, &bytes[offset..]));
        }
    }
    Err(PlyError::MissingEndHeader)
}

/// Parse the header text into the declared element list.
fn parse_header(header: &str) -> Result<Vec<PlyElement>, PlyError> {
    let mut format_seen = false;
    let mut elements: Vec<PlyElement> = Vec::new();

    for line in header.lines().skip(1) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        match tokens.as_slice() {
            ["format", rest @ ..] => {
                format_seen = true;
                if rest != ["binary_little_endian", "1.0"] {
                    return Err(PlyError::UnsupportedFormat(rest.join(" ")));
                }
            }
            ["comment", ..] | ["obj_info", ..] | ["end_header"] => {}
            ["element", name, count] => {
                let count = usize::from_str(count)
                    .map_err(|_| PlyError::InvalidHeader(line.to_string()))?;
                elements.push(PlyElement {
                    name: (*name).to_string(),
                    count,
                    properties: Vec::new(),
                });
            }
            ["property", "list", ..] => {
                return Err(PlyError::UnsupportedListProperty(line.to_string()));
            }
            ["property", type_name, name] => {
                let size = scalar_size(type_name)?;
                let element = elements
                    .last_mut()
                    .ok_or_else(|| PlyError::InvalidHeader(line.to_string()))?;
                element.properties.push((size, (*name).to_string()));
            }
            [] => {}
            _ => return Err(PlyError::InvalidHeader(line.to_string())),
        }
    }

    if !format_seen {
        return Err(PlyError::UnsupportedFormat(String::new()));
    }
    Ok(elements)
}

/// Look up the column index of a required vertex property by name.
fn column(columns: &HashMap<&str, usize>, name: &str) -> Result<usize, PlyError> {
    columns
        .get(name)
        .copied()
        .ok_or_else(|| PlyError::MissingProperty(name.to_string()))
}

/// Look up the column indices of `N` required vertex properties at once.
fn columns_n<const N: usize>(
    columns: &HashMap<&str, usize>,
    names: [&str; N],
) -> Result<[usize; N], PlyError> {
    let mut out = [0usize; N];
    for (slot, name) in out.iter_mut().zip(names) {
        *slot = column(columns, name)?;
    }
    Ok(out)
}

/// Parse a binary little-endian PLY byte stream into a [`GaussianScene`].
pub fn parse_ply(bytes: &[u8]) -> Result<GaussianScene, PlyError> {
    let (header, mut body) = split_header(bytes)?;
    let elements = parse_header(header)?;

    let mut scene = GaussianScene::default();
    for element in &elements {
        let stride: usize = element.properties.iter().map(|(size, _)| size).sum();
        if element.name != "vertex" {
            // Skip over non-vertex elements (uncommon in 3DGS exports).
            let total = stride
                .checked_mul(element.count)
                .ok_or(PlyError::UnexpectedEof)?;
            body = body.get(total..).ok_or(PlyError::UnexpectedEof)?;
            continue;
        }

        // Vertex properties must all be `float` (4 bytes) in a 3DGS export.
        let columns: HashMap<&str, usize> = element
            .properties
            .iter()
            .enumerate()
            .map(|(i, (size, name))| {
                if *size != 4 {
                    return Err(PlyError::UnsupportedPropertyType(name.clone()));
                }
                Ok((name.as_str(), i))
            })
            .collect::<Result<_, _>>()?;

        // Resolve every required column up-front so a bad file fails before
        // any partial scene is produced.
        let [ix, iy, iz] = columns_n(&columns, ["x", "y", "z"])?;
        let iopacity = column(&columns, "opacity")?;
        let iscale = columns_n(&columns, ["scale_0", "scale_1", "scale_2"])?;
        let irot = columns_n(&columns, ["rot_0", "rot_1", "rot_2", "rot_3"])?;
        let idc = columns_n(&columns, ["f_dc_0", "f_dc_1", "f_dc_2"])?;
        let irest: Vec<usize> = (0..SH_REST_LEN)
            .map(|i| column(&columns, &format!("f_rest_{i}")))
            .collect::<Result<_, _>>()?;

        let width = element.properties.len();
        if width > 64 {
            return Err(PlyError::InvalidHeader(format!(
                "vertex element has {width} properties (max 64)"
            )));
        }
        for _ in 0..element.count {
            let record = body.get(..stride).ok_or(PlyError::UnexpectedEof)?;
            body = &body[stride..];
            let mut row = [0.0f32; 64];
            for (i, cell) in row.iter_mut().enumerate().take(width) {
                *cell = f32::from_le_bytes(record[i * 4..i * 4 + 4].try_into().unwrap());
            }

            scene.positions.push([row[ix], row[iy], row[iz]]);
            scene.opacities.push(row[iopacity]);
            scene.scales.push(iscale.map(|i| row[i]));
            scene.rotations.push(irot.map(|i| row[i]));
            scene.sh_dc.push(idc.map(|i| row[i]));
            let mut sh_rest = [0.0f32; SH_REST_LEN];
            for (c, &i) in irest.iter().enumerate() {
                sh_rest[c] = row[i];
            }
            scene.sh_rest.push(sh_rest);
        }
    }

    if scene.is_empty() && !elements.iter().any(|e| e.name == "vertex") {
        return Err(PlyError::MissingVertexElement);
    }
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 3-vertex binary LE PLY with a deliberately shuffled property
    /// order (opacity first, scales before positions, normals at the end) to
    /// prove values are resolved by name, not position.
    fn sample_ply() -> Vec<u8> {
        let props: Vec<String> = ["opacity"]
            .into_iter()
            .map(str::to_string)
            .chain((0..3).map(|i| format!("scale_{i}")))
            .chain(["x", "y", "z"].map(str::to_string))
            .chain((0..4).map(|i| format!("rot_{i}")))
            .chain((0..3).map(|i| format!("f_dc_{i}")))
            .chain((0..45).map(|i| format!("f_rest_{i}")))
            .chain(["nx", "ny", "nz"].map(str::to_string))
            .collect();

        let mut out = b"ply\nformat binary_little_endian 1.0\ncomment test fixture\n".to_vec();
        out.extend_from_slice(b"element vertex 3\n");
        for p in &props {
            out.extend_from_slice(format!("property float {p}\n").as_bytes());
        }
        out.extend_from_slice(b"end_header\n");

        // Vertex v: each property gets a unique, predictable value.
        for v in 0..3u32 {
            for (i, _) in props.iter().enumerate() {
                let value = (v * 100 + i as u32) as f32 + 0.5;
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn test_parse_ply_resolves_properties_by_name() {
        let scene = parse_ply(&sample_ply()).unwrap();
        assert_eq!(scene.len(), 3);

        // Property indices in the shuffled header.
        let idx = |name: &str| -> usize {
            match name {
                "opacity" => 0,
                n if n.starts_with("scale_") => 1 + n[6..].parse::<usize>().unwrap(),
                "x" => 4,
                "y" => 5,
                "z" => 6,
                n if n.starts_with("rot_") => 7 + n[4..].parse::<usize>().unwrap(),
                n if n.starts_with("f_dc_") => 11 + n[5..].parse::<usize>().unwrap(),
                n if n.starts_with("f_rest_") => 14 + n[7..].parse::<usize>().unwrap(),
                _ => unreachable!(),
            }
        };
        let value = |v: usize, name: &str| (v * 100 + idx(name)) as f32 + 0.5;

        for v in 0..3 {
            assert_eq!(
                scene.positions[v],
                [value(v, "x"), value(v, "y"), value(v, "z")]
            );
            assert_eq!(scene.opacities[v], value(v, "opacity"));
            assert_eq!(
                scene.scales[v],
                [
                    value(v, "scale_0"),
                    value(v, "scale_1"),
                    value(v, "scale_2")
                ]
            );
            assert_eq!(
                scene.rotations[v],
                [
                    value(v, "rot_0"),
                    value(v, "rot_1"),
                    value(v, "rot_2"),
                    value(v, "rot_3")
                ]
            );
            assert_eq!(
                scene.sh_dc[v],
                [value(v, "f_dc_0"), value(v, "f_dc_1"), value(v, "f_dc_2")]
            );
            for c in 0..45 {
                assert_eq!(scene.sh_rest[v][c], value(v, &format!("f_rest_{c}")));
            }
        }
    }

    #[test]
    fn test_parse_ply_rejects_ascii_format() {
        let bytes = b"ply\nformat ascii 1.0\nelement vertex 0\nend_header\n";
        assert_eq!(
            parse_ply(bytes),
            Err(PlyError::UnsupportedFormat("ascii 1.0".to_string()))
        );
    }

    #[test]
    fn test_parse_ply_rejects_missing_end_header() {
        let bytes = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty float x\n";
        assert_eq!(parse_ply(bytes), Err(PlyError::MissingEndHeader));
    }

    #[test]
    fn test_parse_ply_rejects_truncated_body() {
        let mut bytes = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\n".to_vec();
        bytes.extend_from_slice(b"property float x\nproperty float y\nproperty float z\n");
        bytes.extend_from_slice(b"property float opacity\n");
        for i in 0..3 {
            bytes.extend_from_slice(format!("property float scale_{i}\n").as_bytes());
        }
        for i in 0..4 {
            bytes.extend_from_slice(format!("property float rot_{i}\n").as_bytes());
        }
        for i in 0..3 {
            bytes.extend_from_slice(format!("property float f_dc_{i}\n").as_bytes());
        }
        for i in 0..45 {
            bytes.extend_from_slice(format!("property float f_rest_{i}\n").as_bytes());
        }
        bytes.extend_from_slice(b"end_header\n");
        bytes.extend_from_slice(&[0u8; 8]); // far short of one vertex
        assert_eq!(parse_ply(&bytes), Err(PlyError::UnexpectedEof));
    }
}
