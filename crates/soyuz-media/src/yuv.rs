//! Planar YUV pictures for a GPU colour conversion (V4b).
//!
//! When a [`crate::FrameSink`] asks for YUV ([`crate::FrameSink::wants_yuv`]), the
//! player hands it the decoded planes ([`YuvFrame`]: I420 or NV12, 8 bits or 10 / 16
//! bits in 16-bit samples) instead of RGBA, with the colour description of the frame
//! ([`YuvColor`]). The frontend uploads the planes as integer textures and converts on
//! the GPU (`soyuz-app` `ui/preview/video_gpu.rs`) with the affine matrix of
//! [`Conversion::of`], then applies the transfer: SDR as is, PQ / HLG tone-mapped to SDR.
//!
//! [`YuvImage::to_rgba`] is the CPU reference of the same arithmetic (the shader mirrors
//! it line by line): the tests compare it with swscale and with known colour values.

/// How the chroma planes are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvLayout {
    /// Three planes: Y, then U and V at half width and half height.
    I420,
    /// Two planes: Y, then interleaved U / V pairs at half width and half height.
    Nv12,
}

/// The YCbCr → RGB matrix (`AVColorSpace`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum YuvMatrix {
    /// BT.601 (SD; also JPEG).
    #[default]
    Bt601,
    Bt709,
    /// BT.2020 non-constant luminance.
    Bt2020,
}

impl YuvMatrix {
    /// `(Kr, Kb)`.
    pub fn coefficients(self) -> (f64, f64) {
        match self {
            YuvMatrix::Bt601 => (0.299, 0.114),
            YuvMatrix::Bt709 => (0.2126, 0.0722),
            YuvMatrix::Bt2020 => (0.2627, 0.0593),
        }
    }
}

/// The transfer function of the decoded values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Transfer {
    /// Gamma-encoded SDR (BT.709 / sRGB / BT.601): shown as is.
    #[default]
    Sdr,
    /// SMPTE ST 2084 (HDR10).
    Pq,
    /// ARIB STD-B67 (hybrid log-gamma).
    Hlg,
}

/// The colour description of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct YuvColor {
    pub matrix: YuvMatrix,
    /// Full ("JPEG", 0–255) range instead of limited (16–235 / 16–240).
    pub full_range: bool,
    pub transfer: Transfer,
    /// BT.2020 primaries (converted to BT.709 after linearising HDR).
    pub bt2020_primaries: bool,
}

/// One decoded picture as planes (borrowed from the decoder's frame for the duration of
/// [`crate::FrameSink::present_yuv`]).
#[derive(Clone, Copy, Debug)]
pub struct YuvFrame<'a> {
    /// Luma size in samples.
    pub width: u32,
    pub height: u32,
    pub layout: YuvLayout,
    /// Bytes per sample: 1, or 2 (little-endian 16-bit containers).
    pub bytes: u8,
    /// Significant bits per sample (8, 10, 12, 16).
    pub depth: u8,
    /// The significant bits sit at the top of the 16-bit container (P010).
    pub msb: bool,
    /// Y, U (or UV), V (empty for NV12); each at least `stride * (rows - 1) + row bytes`.
    pub planes: [&'a [u8]; 3],
    pub strides: [usize; 3],
    pub color: YuvColor,
    /// Shown size before rotation (the sample aspect ratio applied).
    pub display: (u32, u32),
    /// Clockwise rotation to apply for display (0 / 90 / 180 / 270).
    pub rotation: i32,
    /// Presentation time (seconds).
    pub pts: f64,
}

/// An owned copy of a [`YuvFrame`] with tightly packed rows.
#[derive(Clone, Debug, PartialEq)]
pub struct YuvImage {
    pub width: u32,
    pub height: u32,
    pub layout: YuvLayout,
    pub bytes: u8,
    pub depth: u8,
    pub msb: bool,
    /// Y, U (or UV), V (empty for NV12), rows of [`plane_size`](Self::plane_size) samples.
    pub planes: [Vec<u8>; 3],
    pub color: YuvColor,
    pub display: (u32, u32),
    pub rotation: i32,
    pub pts: f64,
}

impl YuvImage {
    /// Copy the planes of `f` row by row (dropping the decoder's padding). `None` when a
    /// plane is shorter than its size says.
    pub fn from_frame(f: &YuvFrame<'_>) -> Option<YuvImage> {
        let mut img = YuvImage {
            width: f.width,
            height: f.height,
            layout: f.layout,
            bytes: f.bytes.clamp(1, 2),
            depth: f.depth,
            msb: f.msb,
            planes: [Vec::new(), Vec::new(), Vec::new()],
            color: f.color,
            display: f.display,
            rotation: f.rotation,
            pts: f.pts,
        };
        for i in 0..img.plane_count() {
            let (w, h) = img.plane_size(i);
            let row = w as usize * img.texel_bytes(i);
            let (src, stride) = (f.planes[i], f.strides[i]);
            if h == 0 || stride < row || src.len() < stride * (h as usize - 1) + row {
                return None;
            }
            let dst = &mut img.planes[i];
            dst.reserve_exact(row * h as usize);
            for y in 0..h as usize {
                dst.extend_from_slice(&src[y * stride..y * stride + row]);
            }
        }
        Some(img)
    }

    /// 3 for I420, 2 for NV12.
    pub fn plane_count(&self) -> usize {
        match self.layout {
            YuvLayout::I420 => 3,
            YuvLayout::Nv12 => 2,
        }
    }

    /// Size of plane `i` in texels (a texel of the NV12 chroma plane is a U / V pair).
    pub fn plane_size(&self, i: usize) -> (u32, u32) {
        if i == 0 { (self.width, self.height) } else { (self.width.div_ceil(2), self.height.div_ceil(2)) }
    }

    /// Bytes of one texel of plane `i`.
    pub fn texel_bytes(&self, i: usize) -> usize {
        let pair = usize::from(i == 1 && self.layout == YuvLayout::Nv12) + 1;
        usize::from(self.bytes) * pair
    }

    /// The displayed size after rotation.
    pub fn shown_size(&self) -> (u32, u32) {
        let (w, h) = self.display;
        if self.rotation == 90 || self.rotation == 270 { (h, w) } else { (w, h) }
    }

    /// Raw sample `c` (0 = first component) of plane `i` at texel `(x, y)`.
    fn raw(&self, i: usize, x: u32, y: u32, c: usize) -> u32 {
        let (w, _) = self.plane_size(i);
        let tb = self.texel_bytes(i);
        let at = (y as usize * w as usize + x as usize) * tb + c * usize::from(self.bytes);
        let p = &self.planes[i];
        if self.bytes == 2 { u32::from(u16::from_le_bytes([p[at], p[at + 1]])) } else { u32::from(p[at]) }
    }

    /// Y, Cb, Cr raw values of the luma sample `(x, y)` (chroma nearest).
    fn ycbcr(&self, x: u32, y: u32) -> [u32; 3] {
        let (cx, cy) = (x / 2, y / 2);
        let yv = self.raw(0, x, y, 0);
        match self.layout {
            YuvLayout::I420 => [yv, self.raw(1, cx, cy, 0), self.raw(2, cx, cy, 0)],
            YuvLayout::Nv12 => [yv, self.raw(1, cx, cy, 0), self.raw(1, cx, cy, 1)],
        }
    }

    /// The conversion constants of this picture.
    pub fn conversion(&self) -> Conversion {
        Conversion::of(self.color, self.bytes, self.depth, self.msb)
    }

    /// CPU reference of the GPU conversion: tightly packed RGBA of the *unrotated*
    /// `width × height` picture (sample aspect ratio not applied), gamma-encoded.
    pub fn to_rgba(&self) -> Vec<u8> {
        let conv = self.conversion();
        let mut out = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for y in 0..self.height {
            for x in 0..self.width {
                let [a, b, c] = self.ycbcr(x, y);
                let rgb = conv.apply([a as f32, b as f32, c as f32]);
                out.extend(rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
                out.push(255);
            }
        }
        out
    }
}

/// The per-picture constants of the conversion (the shader's uniform block).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conversion {
    /// Raw sample → 8-bit code units (`1`, `1/4` for 10 bits, `1/256` for P010).
    pub scale: f32,
    /// Rows of the affine map from 8-bit codes `(Y, Cb, Cr, 1)` to non-linear R'G'B'
    /// (row-major, 3 × 4).
    pub matrix: [[f32; 4]; 3],
    pub transfer: Transfer,
    /// Convert BT.2020 primaries to BT.709 (HDR only; SDR BT.2020 is rare and shown as is).
    pub gamut: bool,
}

/// Reference white of HDR content in nits (BT.2408): PQ / HLG at this level becomes SDR
/// white before tone mapping.
pub const HDR_REFERENCE_WHITE: f32 = 203.0;
/// Peak assumed for PQ content without metadata (nits).
pub const PQ_ASSUMED_PEAK: f32 = 1000.0;

impl Conversion {
    pub fn of(color: YuvColor, bytes: u8, depth: u8, msb: bool) -> Conversion {
        let scale = if bytes >= 2 {
            let shift = if msb { 8 } else { i32::from(depth.clamp(8, 16)) - 8 };
            1.0 / f64::from(1u32 << shift)
        } else {
            1.0
        };
        let (kr, kb) = color.matrix.coefficients();
        let kg = 1.0 - kr - kb;
        // Code → normalised Y' in 0..1 and Cb / Cr in -0.5..0.5.
        let (ys, yo, cs) =
            if color.full_range { (1.0 / 255.0, 0.0, 1.0 / 255.0) } else { (1.0 / 219.0, 16.0, 1.0 / 224.0) };
        // R' = Y + 2(1-Kr) Cr; G' = Y - 2(1-Kb)Kb/Kg Cb - 2(1-Kr)Kr/Kg Cr; B' = Y + 2(1-Kb) Cb.
        let rows = [
            [1.0, 0.0, 2.0 * (1.0 - kr)],
            [1.0, -2.0 * (1.0 - kb) * kb / kg, -2.0 * (1.0 - kr) * kr / kg],
            [1.0, 2.0 * (1.0 - kb), 0.0],
        ];
        let mut matrix = [[0f32; 4]; 3];
        for (m, r) in matrix.iter_mut().zip(rows) {
            let (cy, cb, cr) = (r[0] * ys, r[1] * cs, r[2] * cs);
            let offset = -(cy * yo) - (cb + cr) * 128.0;
            *m = [cy as f32, cb as f32, cr as f32, offset as f32];
        }
        Conversion {
            scale: scale as f32,
            matrix,
            transfer: color.transfer,
            gamut: color.bt2020_primaries && color.transfer != Transfer::Sdr,
        }
    }

    /// Raw `(Y, Cb, Cr)` → gamma-encoded RGB in 0..1 (unclamped for SDR).
    pub fn apply(&self, raw: [f32; 3]) -> [f32; 3] {
        let c = raw.map(|v| v * self.scale);
        let rgb = self.matrix.map(|m| m[0] * c[0] + m[1] * c[1] + m[2] * c[2] + m[3]);
        match self.transfer {
            Transfer::Sdr => rgb,
            Transfer::Pq | Transfer::Hlg => {
                let lin = rgb.map(|v| match self.transfer {
                    Transfer::Pq => pq_eotf(v) / HDR_REFERENCE_WHITE,
                    _ => hlg_to_nits(v) / HDR_REFERENCE_WHITE,
                });
                let lin = if self.gamut { bt2020_to_bt709(lin) } else { lin };
                let peak = match self.transfer {
                    Transfer::Pq => PQ_ASSUMED_PEAK,
                    _ => 1000.0,
                } / HDR_REFERENCE_WHITE;
                tone_map(lin, peak).map(srgb_oetf)
            }
        }
    }
}

/// SMPTE ST 2084 EOTF: signal 0..1 → nits.
pub fn pq_eotf(e: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let p = e.clamp(0.0, 1.0).powf(1.0 / M2);
    let num = (p - C1).max(0.0);
    let den = C2 - C3 * p;
    10_000.0 * (num / den).powf(1.0 / M1)
}

/// HLG inverse OETF plus a simplified OOTF (per component, system gamma 1.2) for a
/// 1000-nit display: signal 0..1 → nits.
pub fn hlg_to_nits(e: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 0.284_668_92;
    const C: f32 = 0.559_910_7;
    let e = e.clamp(0.0, 1.0);
    let scene = if e <= 0.5 { e * e / 3.0 } else { (((e - C) / A).exp() + B) / 12.0 };
    1000.0 * scene.powf(1.2)
}

/// Linear BT.2020 → linear BT.709 (BT.2087).
pub fn bt2020_to_bt709(c: [f32; 3]) -> [f32; 3] {
    [
        1.6605 * c[0] - 0.5876 * c[1] - 0.0728 * c[2],
        -0.1246 * c[0] + 1.1329 * c[1] - 0.0083 * c[2],
        -0.0182 * c[0] - 0.1006 * c[1] + 1.1187 * c[2],
    ]
}

/// Extended Reinhard on the maximum component (keeps hue): `peak` maps to 1.0.
pub fn tone_map(c: [f32; 3], peak: f32) -> [f32; 3] {
    let c = c.map(|v| v.max(0.0));
    let m = c[0].max(c[1]).max(c[2]);
    if m <= 1e-6 {
        return [0.0; 3];
    }
    let w2 = peak.max(1.0) * peak.max(1.0);
    let mapped = m * (1.0 + m / w2) / (1.0 + m);
    c.map(|v| v * mapped / m)
}

/// sRGB OETF: linear 0..1 → gamma-encoded.
pub fn srgb_oetf(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(matrix: YuvMatrix, full: bool) -> Conversion {
        Conversion::of(YuvColor { matrix, full_range: full, ..Default::default() }, 1, 8, false)
    }

    fn rgb8(c: &Conversion, y: u32, u: u32, v: u32) -> [i32; 3] {
        c.apply([y as f32, u as f32, v as f32]).map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as i32)
    }

    fn close(a: [i32; 3], b: [i32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 2)
    }

    #[test]
    fn limited_range_black_white_and_primaries() {
        for m in [YuvMatrix::Bt601, YuvMatrix::Bt709, YuvMatrix::Bt2020] {
            let c = conv(m, false);
            assert_eq!(rgb8(&c, 16, 128, 128), [0, 0, 0], "{m:?}");
            assert_eq!(rgb8(&c, 235, 128, 128), [255, 255, 255], "{m:?}");
        }
        // BT.709 limited 75 % bars (ITU-R BT.709 / SMPTE RP 219 code values).
        let c = conv(YuvMatrix::Bt709, false);
        assert!(close(rgb8(&c, 63, 102, 240), [255, 0, 0]), "{:?}", rgb8(&c, 63, 102, 240));
        assert!(close(rgb8(&c, 173, 42, 26), [0, 255, 0]), "{:?}", rgb8(&c, 173, 42, 26));
        assert!(close(rgb8(&c, 32, 240, 118), [0, 0, 255]), "{:?}", rgb8(&c, 32, 240, 118));
        // BT.601 limited: red is (81, 90, 240).
        let c = conv(YuvMatrix::Bt601, false);
        assert!(close(rgb8(&c, 81, 90, 240), [255, 0, 0]), "{:?}", rgb8(&c, 81, 90, 240));
        // Full range (JPEG): grey stays grey, red is (76, 85, 255).
        let c = conv(YuvMatrix::Bt601, true);
        assert_eq!(rgb8(&c, 0, 128, 128), [0, 0, 0]);
        assert_eq!(rgb8(&c, 255, 128, 128), [255, 255, 255]);
        assert!(close(rgb8(&c, 76, 85, 255), [255, 0, 0]), "{:?}", rgb8(&c, 76, 85, 255));
    }

    #[test]
    fn ten_bit_samples_scale_to_eight_bit_codes() {
        let color = YuvColor { matrix: YuvMatrix::Bt709, ..Default::default() };
        let lsb = Conversion::of(color, 2, 10, false);
        let msb = Conversion::of(color, 2, 10, true);
        assert_eq!(lsb.scale, 0.25);
        assert_eq!(msb.scale, 1.0 / 256.0);
        // 10-bit white (940) and P010 white (940 << 6) are 8-bit 235.
        assert_eq!(rgb8(&lsb, 940, 512, 512), [255, 255, 255]);
        assert_eq!(rgb8(&msb, 940 << 6, 512 << 6, 512 << 6), [255, 255, 255]);
        assert_eq!(rgb8(&lsb, 64, 512, 512), [0, 0, 0]);
        assert_eq!(Conversion::of(color, 1, 8, false).scale, 1.0);
    }

    #[test]
    fn hdr_transfers_tone_map_into_sdr() {
        // PQ: 203 nits (reference white) is code ~0.58; 10 000 nits is 1.0.
        assert!((pq_eotf(1.0) - 10_000.0).abs() < 1.0);
        assert!(pq_eotf(0.0) < 1e-3);
        assert!((pq_eotf(0.5807) - 203.0).abs() < 3.0, "{}", pq_eotf(0.5807));
        assert!((hlg_to_nits(1.0) - 1000.0).abs() < 1.0);
        // Monotonic, at most 1 up to the peak (above it the output clips), black stays
        // black, hue kept.
        let mut last = -1.0;
        for i in 0..=49 {
            let x = i as f32 * 0.1;
            let v = tone_map([x, x * 0.5, 0.0], 1000.0 / 203.0);
            assert!(v[0] >= last && v[0] <= 1.0);
            assert!((v[1] - v[0] * 0.5).abs() < 1e-5);
            last = v[0];
        }
        assert_eq!(tone_map([0.0; 3], 4.0), [0.0; 3]);
        assert!((tone_map([1000.0 / 203.0; 3], 1000.0 / 203.0)[0] - 1.0).abs() < 1e-4, "peak maps to white");
        // A PQ grey at reference white comes out a light grey, not clipped or black.
        let color = YuvColor {
            matrix: YuvMatrix::Bt2020,
            transfer: Transfer::Pq,
            bt2020_primaries: true,
            ..Default::default()
        };
        let c = Conversion::of(color, 2, 10, false);
        assert!(c.gamut);
        // 10-bit limited code of signal 0.58: 64 + 0.58 * 876.
        let code = 64.0 + 0.5807 * 876.0;
        let [r, g, b] = c.apply([code, 512.0, 512.0]);
        assert!((r - g).abs() < 1e-3 && (g - b).abs() < 1e-3, "grey stays grey");
        assert!((0.55..0.9).contains(&r), "{r}");
        // White (Y code 940) is at the 10 000-nit ceiling: mapped to 1.0.
        assert!(c.apply([940.0, 512.0, 512.0])[0] > 0.99);
        // BT.2020 white is BT.709 white.
        let w = bt2020_to_bt709([1.0; 3]);
        assert!(w.iter().all(|v| (v - 1.0).abs() < 2e-3), "{w:?}");
        assert!((srgb_oetf(1.0) - 1.0).abs() < 1e-6);
        assert!((srgb_oetf(0.18) - 0.461).abs() < 2e-3);
    }

    #[test]
    fn images_copy_rows_and_convert() {
        // 3 × 2 I420 with padded strides; chroma 2 × 1.
        let y = [16u8, 128, 235, 0, 16, 128, 235, 0];
        let u = [128u8, 128, 9];
        let v = [128u8, 128, 9];
        let f = YuvFrame {
            width: 3,
            height: 2,
            layout: YuvLayout::I420,
            bytes: 1,
            depth: 8,
            msb: false,
            planes: [&y, &u, &v],
            strides: [4, 3, 3],
            color: YuvColor::default(),
            display: (3, 2),
            rotation: 90,
            pts: 1.5,
        };
        let img = YuvImage::from_frame(&f).unwrap();
        assert_eq!(img.planes[0], [16, 128, 235, 16, 128, 235]);
        assert_eq!(img.planes[1], [128, 128]);
        assert_eq!(img.plane_size(1), (2, 1));
        assert_eq!(img.shown_size(), (2, 3));
        let rgba = img.to_rgba();
        assert_eq!(rgba.len(), 3 * 2 * 4);
        assert_eq!(&rgba[..4], &[0, 0, 0, 255]);
        assert_eq!(&rgba[8..12], &[255, 255, 255, 255]);
        // NV12 with 16-bit samples: the UV plane has 4-byte texels.
        let y16: Vec<u8> = [940u16, 940, 64, 64].iter().flat_map(|v| v.to_le_bytes()).collect();
        let uv16: Vec<u8> = [512u16, 512].iter().flat_map(|v| v.to_le_bytes()).collect();
        let f = YuvFrame {
            width: 2,
            height: 2,
            layout: YuvLayout::Nv12,
            bytes: 2,
            depth: 10,
            msb: false,
            planes: [&y16, &uv16, &[]],
            strides: [4, 4, 0],
            color: YuvColor { matrix: YuvMatrix::Bt709, ..Default::default() },
            display: (2, 2),
            rotation: 0,
            pts: 0.0,
        };
        let img = YuvImage::from_frame(&f).unwrap();
        assert_eq!(img.plane_count(), 2);
        assert_eq!(img.texel_bytes(1), 4);
        let rgba = img.to_rgba();
        assert_eq!(&rgba[..4], &[255, 255, 255, 255]);
        assert_eq!(&rgba[8..12], &[0, 0, 0, 255]);
        // A plane shorter than its size is refused.
        let short = YuvFrame { planes: [&y16[..6], &uv16, &[]], ..f };
        assert!(YuvImage::from_frame(&short).is_none());
    }
}
