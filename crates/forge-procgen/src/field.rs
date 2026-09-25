//! A square grid of samples with a spacing in metres: heights, uplift, flow, layers.

/// `size × size` samples, `spacing` metres apart, row-major (`y` rows along +z, `x` along +x),
/// the grid's corner sample at the origin of its frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Field2<T> {
    /// Samples per side.
    pub size: u32,
    /// Metres between samples.
    pub spacing: f64,
    /// The samples, row-major.
    pub data: Vec<T>,
}

impl<T: Copy + Default> Field2<T> {
    /// A field of default values.
    pub fn new(size: u32, spacing: f64) -> Self {
        Self {
            size,
            spacing,
            data: vec![T::default(); (size as usize) * (size as usize)],
        }
    }

    /// A field filled by `f(x, y)` over the sample indices.
    pub fn from_fn(size: u32, spacing: f64, mut f: impl FnMut(u32, u32) -> T) -> Self {
        let mut data = Vec::with_capacity((size as usize) * (size as usize));
        for y in 0..size {
            for x in 0..size {
                data.push(f(x, y));
            }
        }
        Self {
            size,
            spacing,
            data,
        }
    }

    /// The index of sample (x, y).
    #[inline]
    pub fn index(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.size as usize) + x as usize
    }

    /// The coordinates of index `i`.
    #[inline]
    pub fn coords(&self, i: usize) -> (u32, u32) {
        (
            (i % self.size as usize) as u32,
            (i / self.size as usize) as u32,
        )
    }

    /// The sample at (x, y).
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> T {
        self.data[self.index(x, y)]
    }

    /// Sets the sample at (x, y).
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, value: T) {
        let i = self.index(x, y);
        self.data[i] = value;
    }

    /// How many samples.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the field has no samples.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The side of the field, metres (between the first and the last sample).
    pub fn extent(&self) -> f64 {
        f64::from(self.size.saturating_sub(1)) * self.spacing
    }

    /// A field of the same shape, each sample mapped.
    pub fn map<U: Copy + Default>(&self, f: impl FnMut(T) -> U) -> Field2<U> {
        Field2 {
            size: self.size,
            spacing: self.spacing,
            data: self.data.iter().copied().map(f).collect(),
        }
    }

    /// Whether (x, y) lies on the outer ring of samples.
    #[inline]
    pub fn on_border(&self, x: u32, y: u32) -> bool {
        x == 0 || y == 0 || x + 1 == self.size || y + 1 == self.size
    }
}

impl Field2<f32> {
    /// A 64-bit FNV-1a digest of the samples' bits (little-endian), the same on every machine
    /// for the same field: what two runs compare to check D-016.
    pub fn digest(&self) -> u64 {
        self.data.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, v| {
            v.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
    }

    /// The smallest and largest sample.
    pub fn min_max(&self) -> (f32, f32) {
        self.data
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)))
    }

    /// The sample interpolated at (`x`, `y`) metres, clamped to the field.
    pub fn sample(&self, x: f64, y: f64) -> f32 {
        let max = f64::from(self.size - 1) - 1e-9;
        let gx = (x / self.spacing).clamp(0.0, max);
        let gy = (y / self.spacing).clamp(0.0, max);
        let (i, j) = (gx.floor() as u32, gy.floor() as u32);
        let (fx, fy) = ((gx - f64::from(i)) as f32, (gy - f64::from(j)) as f32);
        let (i1, j1) = ((i + 1).min(self.size - 1), (j + 1).min(self.size - 1));
        let top = self.get(i, j) + (self.get(i1, j) - self.get(i, j)) * fx;
        let bottom = self.get(i, j1) + (self.get(i1, j1) - self.get(i, j1)) * fx;
        top + (bottom - top) * fy
    }

    /// The gradient (∂/∂x, ∂/∂y) at (x, y) by Horn's 3 × 3 kernel, per metre; the border
    /// uses the samples it has.
    pub fn gradient(&self, x: u32, y: u32) -> (f32, f32) {
        let n = self.size - 1;
        let at = |dx: i32, dy: i32| {
            let sx = (x as i32 + dx).clamp(0, n as i32) as u32;
            let sy = (y as i32 + dy).clamp(0, n as i32) as u32;
            self.get(sx, sy)
        };
        let dx =
            (at(1, -1) + 2.0 * at(1, 0) + at(1, 1)) - (at(-1, -1) + 2.0 * at(-1, 0) + at(-1, 1));
        let dy =
            (at(-1, 1) + 2.0 * at(0, 1) + at(1, 1)) - (at(-1, -1) + 2.0 * at(0, -1) + at(1, -1));
        let scale = 1.0 / (8.0 * self.spacing as f32);
        (dx * scale, dy * scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_indexes_samples_and_interpolates_between_them() {
        let f = Field2::from_fn(3, 10.0, |x, y| (x + 10 * y) as f32);
        assert_eq!(f.get(2, 1), 12.0);
        assert_eq!(f.coords(f.index(2, 1)), (2, 1));
        assert_eq!(f.extent(), 20.0);
        assert_eq!(f.sample(5.0, 0.0), 0.5);
        assert_eq!(f.sample(10.0, 15.0), 16.0);
        assert_eq!(f.sample(1e9, -5.0), 2.0);
        assert!(f.on_border(0, 1) && !f.on_border(1, 1));
        // A plane rising 1 m per metre along x has that gradient everywhere inside.
        let plane = Field2::from_fn(5, 2.0, |x, _| 2.0 * x as f32);
        assert_eq!(plane.gradient(2, 2), (1.0, 0.0));
        assert_eq!(plane.min_max(), (0.0, 8.0));
        // The digest tells fields apart and is a constant of the bytes.
        assert_eq!(plane.digest(), plane.clone().digest());
        assert_ne!(plane.digest(), f.digest());
        assert_eq!(Field2::<f32>::new(0, 1.0).digest(), 0xcbf2_9ce4_8422_2325);
    }
}
