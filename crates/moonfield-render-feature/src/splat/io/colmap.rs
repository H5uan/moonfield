//! COLMAP *text* format parsers (`cameras.txt`, `images.txt`, `points3D.txt`),
//! used to bootstrap splat training from a COLMAP reconstruction.
//!
//! Only the plain-text model is supported; COLMAP's binary files (`.bin`)
//! are left for a future milestone. All parsers take `&str` so tests can run
//! on in-memory samples without touching the filesystem.

/// Errors returned by the COLMAP text parsers.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ColmapError {
    /// A camera model other than `PINHOLE` / `SIMPLE_PINHOLE` was found.
    #[error("unsupported camera model `{0}`")]
    UnsupportedCameraModel(String),
    /// A record line has too few fields or is otherwise malformed.
    #[error("invalid line: {0}")]
    InvalidLine(String),
    /// A numeric field failed to parse.
    #[error("invalid number in line: {0}")]
    InvalidNumber(String),
    /// An image record is missing its trailing 2D-points line.
    #[error("missing 2D points line after image {0}")]
    MissingPoints2D(u32),
}

/// Supported COLMAP camera models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraModel {
    /// `SIMPLE_PINHOLE` — shared focal length: `f, cx, cy`.
    SimplePinhole,
    /// `PINHOLE` — separate focal lengths: `fx, fy, cx, cy`.
    Pinhole,
}

/// One camera from `cameras.txt`, with intrinsics normalized to
/// `fx, fy, cx, cy` regardless of the source model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColmapCamera {
    pub id: u32,
    pub model: CameraModel,
    pub width: u32,
    pub height: u32,
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
}

/// One registered image from `images.txt`: world-from-camera pose is the
/// inverse of the stored camera-from-world quaternion + translation.
#[derive(Debug, Clone, PartialEq)]
pub struct ColmapImage {
    pub id: u32,
    /// Camera-from-world rotation quaternion `(qw, qx, qy, qz)`.
    pub rotation: [f64; 4],
    /// Camera-from-world translation `(tx, ty, tz)`.
    pub translation: [f64; 3],
    pub camera_id: u32,
    pub name: String,
}

/// One sparse point from `points3D.txt` (the per-point track is dropped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColmapPoint3D {
    pub position: [f64; 3],
    pub rgb: [u8; 3],
}

fn parse_f64(token: &str, line: &str) -> Result<f64, ColmapError> {
    token
        .parse()
        .map_err(|_| ColmapError::InvalidNumber(line.to_string()))
}

fn parse_u32(token: &str, line: &str) -> Result<u32, ColmapError> {
    token
        .parse()
        .map_err(|_| ColmapError::InvalidNumber(line.to_string()))
}

/// Iterate over meaningful lines: skips blank lines and `#` comments.
fn data_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// Parse `cameras.txt` contents. Camera models other than `PINHOLE` and
/// `SIMPLE_PINHOLE` are rejected with
/// [`ColmapError::UnsupportedCameraModel`].
pub fn parse_cameras(text: &str) -> Result<Vec<ColmapCamera>, ColmapError> {
    let mut cameras = Vec::new();
    for line in data_lines(text) {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() < 4 {
            return Err(ColmapError::InvalidLine(line.to_string()));
        }
        let id = parse_u32(t[0], line)?;
        let width = parse_u32(t[2], line)?;
        let height = parse_u32(t[3], line)?;
        let params = |expected: usize| -> Result<Vec<f64>, ColmapError> {
            if t.len() != 4 + expected {
                return Err(ColmapError::InvalidLine(line.to_string()));
            }
            t[4..].iter().map(|p| parse_f64(p, line)).collect()
        };
        let (model, fx, fy, cx, cy) = match t[1] {
            "SIMPLE_PINHOLE" => {
                let p = params(3)?;
                (CameraModel::SimplePinhole, p[0], p[0], p[1], p[2])
            }
            "PINHOLE" => {
                let p = params(4)?;
                (CameraModel::Pinhole, p[0], p[1], p[2], p[3])
            }
            other => return Err(ColmapError::UnsupportedCameraModel(other.to_string())),
        };
        cameras.push(ColmapCamera {
            id,
            model,
            width,
            height,
            fx,
            fy,
            cx,
            cy,
        });
    }
    Ok(cameras)
}

/// Parse `images.txt` contents. Each image is a header line
/// (`IMAGE_ID qw qx qy qz tx ty tz CAMERA_ID NAME`) followed by one
/// 2D-points line, which is consumed and skipped.
pub fn parse_images(text: &str) -> Result<Vec<ColmapImage>, ColmapError> {
    let mut images = Vec::new();
    let mut lines = text.lines().map(str::trim).peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() < 10 {
            return Err(ColmapError::InvalidLine(line.to_string()));
        }
        let id = parse_u32(t[0], line)?;
        let num = |i: usize| parse_f64(t[i], line);
        let image = ColmapImage {
            id,
            rotation: [num(1)?, num(2)?, num(3)?, num(4)?],
            translation: [num(5)?, num(6)?, num(7)?],
            camera_id: parse_u32(t[8], line)?,
            // The file name may contain spaces; rejoin the tail.
            name: t[9..].join(" "),
        };
        // Every image record is followed by its 2D points line (possibly
        // empty), which we skip.
        if lines.next().is_none() {
            return Err(ColmapError::MissingPoints2D(id));
        }
        images.push(image);
    }
    Ok(images)
}

/// Parse `points3D.txt` contents, keeping only `xyz` and `rgb` of each
/// record (`POINT3D_ID X Y Z R G B ERROR TRACK[]`); the reprojection error
/// and track are dropped.
pub fn parse_points3d(text: &str) -> Result<Vec<ColmapPoint3D>, ColmapError> {
    let mut points = Vec::new();
    for line in data_lines(text) {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() < 8 {
            return Err(ColmapError::InvalidLine(line.to_string()));
        }
        points.push(ColmapPoint3D {
            position: [
                parse_f64(t[1], line)?,
                parse_f64(t[2], line)?,
                parse_f64(t[3], line)?,
            ],
            rgb: [
                parse_u32(t[4], line)? as u8,
                parse_u32(t[5], line)? as u8,
                parse_u32(t[6], line)? as u8,
            ],
        });
    }
    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMERAS: &str = "\
# Camera list with one line of data per camera:
#   CAMERA_ID, MODEL, WIDTH, HEIGHT, PARAMS[]
1 SIMPLE_PINHOLE 3072 2304 2559.81 1536 1152
2 PINHOLE 1920 1080 1500.5 1501.25 960.0 540.0
";

    const IMAGES: &str = "\
# Image list with two lines of data per image:
#   IMAGE_ID, QW, QX, QY, QZ, TX, TY, TZ, CAMERA_ID, NAME
#   POINTS2D[] as (X, Y, POINT3D_ID)
1 0.8518 0.0164 0.5039 -0.1429 -0.737 1.029 3.743 1 P1180141.JPG
2362.39 248.50 58396 1784.27 2685.41 -1
2 0.8519 0.0165 0.5038 -0.1428 -0.736 1.028 3.742 1 P1180142.JPG

";

    const POINTS3D: &str = "\
# 3D point list with one line of data per point:
#   POINT3D_ID, X, Y, Z, R, G, B, ERROR, TRACK[]
63390 1.67241 0.29269 0.60972 115 121 122 1.33927 16 6542 15 7345
63391 -1.0 2.5 3.25 255 0 128 0.9 1 0
";

    #[test]
    fn test_parse_cameras() {
        let cameras = parse_cameras(CAMERAS).unwrap();
        assert_eq!(cameras.len(), 2);
        assert_eq!(
            cameras[0],
            ColmapCamera {
                id: 1,
                model: CameraModel::SimplePinhole,
                width: 3072,
                height: 2304,
                fx: 2559.81,
                fy: 2559.81,
                cx: 1536.0,
                cy: 1152.0,
            }
        );
        assert_eq!(cameras[1].model, CameraModel::Pinhole);
        assert_eq!(cameras[1].fx, 1500.5);
        assert_eq!(cameras[1].fy, 1501.25);
    }

    #[test]
    fn test_parse_cameras_rejects_unsupported_model() {
        let text = "1 OPENCV 1920 1080 1500 1500 960 540 0.1 0.01 0 0\n";
        assert_eq!(
            parse_cameras(text),
            Err(ColmapError::UnsupportedCameraModel("OPENCV".to_string()))
        );
    }

    #[test]
    fn test_parse_images() {
        let images = parse_images(IMAGES).unwrap();
        assert_eq!(images.len(), 2);
        let first = &images[0];
        assert_eq!(first.id, 1);
        assert_eq!(first.rotation, [0.8518, 0.0164, 0.5039, -0.1429]);
        assert_eq!(first.translation, [-0.737, 1.029, 3.743]);
        assert_eq!(first.camera_id, 1);
        assert_eq!(first.name, "P1180141.JPG");
        // Second image has an empty 2D points line; parsing must not consume
        // its own header as the next image's points line.
        assert_eq!(images[1].id, 2);
        assert_eq!(images[1].name, "P1180142.JPG");
    }

    #[test]
    fn test_parse_images_missing_points2d_line() {
        let text = "1 1 0 0 0 0 0 0 1 view.jpg\n";
        assert_eq!(parse_images(text), Err(ColmapError::MissingPoints2D(1)));
    }

    #[test]
    fn test_parse_points3d() {
        let points = parse_points3d(POINTS3D).unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].position, [1.67241, 0.29269, 0.60972]);
        assert_eq!(points[0].rgb, [115, 121, 122]);
        assert_eq!(points[1].position, [-1.0, 2.5, 3.25]);
        assert_eq!(points[1].rgb, [255, 0, 128]);
    }
}
