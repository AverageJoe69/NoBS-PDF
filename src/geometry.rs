use serde::{Deserialize, Serialize};

pub const POINTS_PER_INCH: f64 = 72.0;
pub const MM_PER_INCH: f64 = 25.4;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Matrix multiplication in PDF's affine convention: `self * rhs`.
    pub fn concat(self, rhs: Self) -> Self {
        Self {
            a: self.a * rhs.a + self.b * rhs.c,
            b: self.a * rhs.b + self.b * rhs.d,
            c: self.c * rhs.a + self.d * rhs.c,
            d: self.c * rhs.b + self.d * rhs.d,
            e: self.e * rhs.a + self.f * rhs.c + rhs.e,
            f: self.e * rhs.b + self.f * rhs.d + rhs.f,
        }
    }

    /// Lengths of the transformed unit-square axes. Rotation is therefore handled naturally.
    pub fn axis_lengths_pt(self) -> (f64, f64) {
        (self.a.hypot(self.b), self.c.hypot(self.d))
    }

    pub fn transform_point(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    pub fn unit_square_bounds(self) -> [f64; 4] {
        let points = [
            self.transform_point(0.0, 0.0),
            self.transform_point(1.0, 0.0),
            self.transform_point(0.0, 1.0),
            self.transform_point(1.0, 1.0),
        ];
        points.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |mut bounds, &(x, y)| {
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
                bounds
            },
        )
    }
}

pub fn pt_to_mm(points: f64) -> f64 {
    points * MM_PER_INCH / POINTS_PER_INCH
}
pub fn effective_dpi(pixels: u32, displayed_pt: f64) -> Option<f64> {
    (displayed_pt.is_finite() && displayed_pt > 0.0)
        .then_some(pixels as f64 * POINTS_PER_INCH / displayed_pt)
}
pub fn target_pixels(displayed_pt: f64, dpi: u16) -> Option<u32> {
    (displayed_pt.is_finite() && displayed_pt > 0.0).then(|| {
        (displayed_pt / POINTS_PER_INCH * f64::from(dpi))
            .round()
            .max(1.0) as u32
    })
}
