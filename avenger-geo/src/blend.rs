//! Pointwise interpolation between raw projections for pan, zoom, and transitions.
//!
//! `P_t(λ, φ) = (1 − t)·A(λ, φ) + t·B(λ, φ)`. Blending does not guarantee
//! that the result is globally invertible. The inverse returns `None` when
//! the numerical solver does not converge. An anchoring similarity can keep
//! the anchor position, north direction, and local northward scale fixed.

use crate::math::RADIANS;
use crate::raw::{invert_newton, RawProjection};

/// Pointwise blend of two raw projections.
pub struct BlendRaw {
    pub a: Box<dyn RawProjection>,
    pub b: Box<dyn RawProjection>,
    pub t: f64,
}

impl BlendRaw {
    pub fn new(a: Box<dyn RawProjection>, b: Box<dyn RawProjection>, t: f64) -> Self {
        BlendRaw {
            a,
            b,
            t: t.clamp(0.0, 1.0),
        }
    }
}

impl RawProjection for BlendRaw {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        if self.t <= 0.0 {
            return self.a.project(lambda, phi);
        }
        if self.t >= 1.0 {
            return self.b.project(lambda, phi);
        }
        let (ax, ay) = self.a.project(lambda, phi);
        let (bx, by) = self.b.project(lambda, phi);
        (ax + (bx - ax) * self.t, ay + (by - ay) * self.t)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if self.t <= 0.0 {
            return self.a.invert(x, y);
        }
        if self.t >= 1.0 {
            return self.b.invert(x, y);
        }
        // Seed Newton from the dominant endpoint's closed-form inverse.
        let seed = if self.t < 0.5 {
            self.a.invert(x, y)
        } else {
            self.b.invert(x, y)
        };
        match seed {
            Some((sl, sp)) => refine_newton(|l, p| self.project(l, p), x, y, sl, sp)
                .or_else(|| invert_newton(|l, p| self.project(l, p), x, y)),
            None => invert_newton(|l, p| self.project(l, p), x, y),
        }
    }
}

fn refine_newton<F>(project: F, x: f64, y: f64, mut lambda: f64, mut phi: f64) -> Option<(f64, f64)>
where
    F: Fn(f64, f64) -> (f64, f64),
{
    let h = 1e-8;
    for _ in 0..20 {
        let (fx, fy) = project(lambda, phi);
        let ex = fx - x;
        let ey = fy - y;
        if ex.abs() < 1e-12 && ey.abs() < 1e-12 {
            return Some((lambda, phi));
        }
        let (fxl, fyl) = project(lambda + h, phi);
        let (fxp, fyp) = project(lambda, phi + h);
        let a = (fxl - fx) / h;
        let b = (fxp - fx) / h;
        let c = (fyl - fy) / h;
        let d = (fyp - fy) / h;
        let det = a * d - b * c;
        if det.abs() < 1e-15 || !det.is_finite() {
            return None;
        }
        lambda -= (ex * d - ey * b) / det;
        phi -= (ey * a - ex * c) / det;
        if !lambda.is_finite() || !phi.is_finite() {
            return None;
        }
    }
    let (fx, fy) = project(lambda, phi);
    if (fx - x).abs() < 1e-6 && (fy - y).abs() < 1e-6 {
        Some((lambda, phi))
    } else {
        None
    }
}

/// The local planar frame of a raw projection at an anchor point:
/// position plus the image of a small step north and east.
#[derive(Debug, Clone, Copy)]
pub struct LocalFrame {
    pub origin: (f64, f64),
    /// d(position)/d(north), unit: planar units per radian of latitude.
    pub north: (f64, f64),
    /// d(position)/d(east) at the anchor.
    pub east: (f64, f64),
}

/// Numeric local frame at `anchor` (lon/lat degrees).
pub fn local_frame(raw: &dyn RawProjection, anchor_lon: f64, anchor_lat: f64) -> LocalFrame {
    let l = anchor_lon * RADIANS;
    let p = anchor_lat * RADIANS;
    let h = 1e-6;
    let origin = raw.project(l, p);
    let north_p = raw.project(l, p + h);
    let east_p = raw.project(l + h, p);
    LocalFrame {
        origin,
        north: ((north_p.0 - origin.0) / h, (north_p.1 - origin.1) / h),
        east: ((east_p.0 - origin.0) / h, (east_p.1 - origin.1) / h),
    }
}

/// Solve the similarity transform (rotate + uniform scale + translate, in
/// raw planar space, y-up) mapping the blend's local frame at the anchor
/// onto the reference raw's local frame, so that `correct ∘ blend` matches
/// `reference` in position, ground scale, and north direction at the anchor.
///
/// Returned as `(a, b, dx, dy)` for `x' = a·x − b·y + dx`,
/// `y' = b·x + a·y + dy` (plain planar rotation+scale, NO y flip — this
/// composes with raw output before the display affine).
pub fn anchoring_similarity(
    blend: &dyn RawProjection,
    reference: &dyn RawProjection,
    anchor_lon: f64,
    anchor_lat: f64,
) -> PlanarSimilarity {
    let fb = local_frame(blend, anchor_lon, anchor_lat);
    let fr = local_frame(reference, anchor_lon, anchor_lat);

    // Match the north vectors: rotation + scale taking fb.north to fr.north.
    let nb = fb.north;
    let nr = fr.north;
    let nb2 = nb.0 * nb.0 + nb.1 * nb.1;
    let (a, b) = if nb2 > 0.0 {
        // Complex division nr / nb: (a + i b) * nb = nr
        (
            (nr.0 * nb.0 + nr.1 * nb.1) / nb2,
            (nr.1 * nb.0 - nr.0 * nb.1) / nb2,
        )
    } else {
        (1.0, 0.0)
    };
    // Translate so the anchor image matches.
    let rx = a * fb.origin.0 - b * fb.origin.1;
    let ry = b * fb.origin.0 + a * fb.origin.1;
    PlanarSimilarity {
        a,
        b,
        dx: fr.origin.0 - rx,
        dy: fr.origin.1 - ry,
    }
}

/// Like [`anchoring_similarity`], but the target orientation follows the
/// North-up transition: at `t = 0` north at the anchor points
/// wherever the authored projection puts it, and as `t → 1` it eases to
/// straight up (+y in raw space), so the fully-blended map is an
/// ordinary north-up Mercator. Ground scale at the anchor stays the
/// authored ground scale at every `t`, keeping the view's authored-unit
/// domains valid.
pub fn anchoring_similarity_north_up(
    blend: &dyn RawProjection,
    reference: &dyn RawProjection,
    anchor_lon: f64,
    anchor_lat: f64,
    t: f64,
) -> PlanarSimilarity {
    let fb = local_frame(blend, anchor_lon, anchor_lat);
    let fr = local_frame(reference, anchor_lon, anchor_lat);

    let nb = fb.north;
    let nb2 = nb.0 * nb.0 + nb.1 * nb.1;
    let nr_len = (fr.north.0 * fr.north.0 + fr.north.1 * fr.north.1).sqrt();
    let (a, b) = if nb2 > 0.0 && nr_len > 0.0 {
        // Target north: authored magnitude, angle eased from the authored
        // north angle toward straight up (shortest way around).
        let theta_ref = fr.north.1.atan2(fr.north.0);
        let up = std::f64::consts::FRAC_PI_2;
        let mut delta = up - theta_ref;
        while delta > std::f64::consts::PI {
            delta -= 2.0 * std::f64::consts::PI;
        }
        while delta < -std::f64::consts::PI {
            delta += 2.0 * std::f64::consts::PI;
        }
        let theta = theta_ref + t.clamp(0.0, 1.0) * delta;
        let target = (nr_len * theta.cos(), nr_len * theta.sin());
        // Complex division target / nb.
        (
            (target.0 * nb.0 + target.1 * nb.1) / nb2,
            (target.1 * nb.0 - target.0 * nb.1) / nb2,
        )
    } else {
        (1.0, 0.0)
    };
    let rx = a * fb.origin.0 - b * fb.origin.1;
    let ry = b * fb.origin.0 + a * fb.origin.1;
    PlanarSimilarity {
        a,
        b,
        dx: fr.origin.0 - rx,
        dy: fr.origin.1 - ry,
    }
}

/// A planar similarity `x' = a·x − b·y + dx`, `y' = b·x + a·y + dy`
/// (rotation + uniform scale + translation, no reflection).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanarSimilarity {
    pub a: f64,
    pub b: f64,
    pub dx: f64,
    pub dy: f64,
}

impl PlanarSimilarity {
    pub fn identity() -> Self {
        PlanarSimilarity {
            a: 1.0,
            b: 0.0,
            dx: 0.0,
            dy: 0.0,
        }
    }

    pub fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        (
            self.a * p.0 - self.b * p.1 + self.dx,
            self.b * p.0 + self.a * p.1 + self.dy,
        )
    }

    pub fn invert(&self, p: (f64, f64)) -> (f64, f64) {
        let x = p.0 - self.dx;
        let y = p.1 - self.dy;
        let d = self.a * self.a + self.b * self.b;
        ((self.a * x + self.b * y) / d, (self.a * y - self.b * x) / d)
    }

    pub fn is_near_identity(&self) -> bool {
        (self.a - 1.0).abs() < 1e-12
            && self.b.abs() < 1e-12
            && self.dx.abs() < 1e-12
            && self.dy.abs() < 1e-12
    }
}

/// A raw projection wrapped with an anchoring correction. Use as the raw
/// inside a [`crate::projector::Projection`]-built pipeline: everything
/// (clipping, resampling, fit) operates on it unchanged.
///
/// Construct by building the blend and correction separately:
///
/// ```
/// # use avenger_geo::raw::ProjectionKind;
/// # use avenger_geo::blend::{BlendRaw, CorrectedBlendRaw, anchoring_similarity};
/// let authored = ProjectionKind::albers();
/// let t = 0.5;
/// let blend = BlendRaw::new(authored.raw(), ProjectionKind::Mercator.raw(), t);
/// let reference = authored.raw();
/// let correction = anchoring_similarity(&blend, reference.as_ref(), -98.0, 38.5);
/// let corrected = CorrectedBlendRaw { blend, correction };
/// ```
pub struct CorrectedBlendRaw {
    pub blend: BlendRaw,
    pub correction: PlanarSimilarity,
}

impl RawProjection for CorrectedBlendRaw {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        self.correction.apply(self.blend.project(lambda, phi))
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let p = self.correction.invert((x, y));
        self.blend.invert(p.0, p.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{ProjectionKind, RawProjection};

    fn blend_of(kind_a: ProjectionKind, kind_b: ProjectionKind, t: f64) -> BlendRaw {
        BlendRaw::new(kind_a.raw(), kind_b.raw(), t)
    }

    #[test]
    fn endpoints_match_raws() {
        let a = ProjectionKind::albers().raw();
        let b = ProjectionKind::Mercator.raw();
        let b0 = blend_of(ProjectionKind::albers(), ProjectionKind::Mercator, 0.0);
        let b1 = blend_of(ProjectionKind::albers(), ProjectionKind::Mercator, 1.0);
        for &(l, p) in &[(-1.6, 0.7), (0.3, -0.4), (1.0, 1.0)] {
            assert_eq!(b0.project(l, p), a.project(l, p));
            assert_eq!(b1.project(l, p), b.project(l, p));
        }
    }

    #[test]
    fn blend_inverts() {
        for t in [0.25, 0.5, 0.75] {
            let blend = blend_of(ProjectionKind::albers(), ProjectionKind::Mercator, t);
            for &(l, p) in &[(-1.2, 0.6), (0.4, 0.2), (0.9, -0.5)] {
                let (x, y) = blend.project(l, p);
                let (l2, p2) = blend.invert(x, y).expect("invertible");
                assert!(
                    (l2 - l).abs() < 1e-6 && (p2 - p).abs() < 1e-6,
                    "t={t} ({l}, {p}) -> ({l2}, {p2})"
                );
            }
        }
    }

    #[test]
    fn anchoring_fixes_anchor_and_north() {
        let anchor = (-98.0, 38.5); // Kansas
        let a_kind = ProjectionKind::albers();
        let reference = a_kind.raw();
        let ref_frame = local_frame(reference.as_ref(), anchor.0, anchor.1);

        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let blend = BlendRaw::new(a_kind.raw(), ProjectionKind::Mercator.raw(), t);
            let sim = anchoring_similarity(&blend, reference.as_ref(), anchor.0, anchor.1);
            let corrected = CorrectedBlendRaw {
                blend,
                correction: sim,
            };
            let frame = local_frame(&corrected, anchor.0, anchor.1);
            // Anchor position fixed.
            assert!(
                (frame.origin.0 - ref_frame.origin.0).abs() < 1e-6
                    && (frame.origin.1 - ref_frame.origin.1).abs() < 1e-6,
                "t={t}: origin drifted: {:?} vs {:?}",
                frame.origin,
                ref_frame.origin
            );
            // North direction and scale fixed.
            assert!(
                (frame.north.0 - ref_frame.north.0).abs() < 1e-4
                    && (frame.north.1 - ref_frame.north.1).abs() < 1e-4,
                "t={t}: north drifted: {:?} vs {:?}",
                frame.north,
                ref_frame.north
            );
        }
    }

    #[test]
    fn north_up_anchoring_straightens_as_t_reaches_one() {
        // California-ish anchor, well west of the Albers central meridian,
        // where meridian convergence tilts the authored local north.
        let anchor = (-122.0, 37.0);
        let a_kind = ProjectionKind::albers();
        let reference = a_kind.raw();
        let ref_frame = local_frame(reference.as_ref(), anchor.0, anchor.1);
        let ref_len = (ref_frame.north.0.powi(2) + ref_frame.north.1.powi(2)).sqrt();
        assert!(
            ref_frame.north.0.abs() > 1e-3,
            "fixture should have tilted authored north, got {:?}",
            ref_frame.north
        );

        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let blend = BlendRaw::new(a_kind.raw(), ProjectionKind::Mercator.raw(), t);
            let sim =
                anchoring_similarity_north_up(&blend, reference.as_ref(), anchor.0, anchor.1, t);
            let corrected = CorrectedBlendRaw {
                blend,
                correction: sim,
            };
            let frame = local_frame(&corrected, anchor.0, anchor.1);
            // Anchor position fixed at every t.
            assert!(
                (frame.origin.0 - ref_frame.origin.0).abs() < 1e-6
                    && (frame.origin.1 - ref_frame.origin.1).abs() < 1e-6,
                "t={t}: origin drifted"
            );
            // Ground scale (|north|) preserved at every t.
            let len = (frame.north.0.powi(2) + frame.north.1.powi(2)).sqrt();
            assert!(
                (len - ref_len).abs() / ref_len < 1e-4,
                "t={t}: north magnitude drifted: {len} vs {ref_len}"
            );
            if t == 0.0 {
                // Authored orientation at the start.
                assert!(
                    (frame.north.0 - ref_frame.north.0).abs() < 1e-6
                        && (frame.north.1 - ref_frame.north.1).abs() < 1e-6,
                    "t=0 should preserve the authored frame"
                );
            }
            if t == 1.0 {
                // Fully blended: north points straight up.
                assert!(
                    frame.north.0.abs() < 1e-6 && frame.north.1 > 0.0,
                    "t=1 should be north-up, got north {:?}",
                    frame.north
                );
            }
        }
    }

    #[test]
    fn anchoring_at_t0_is_identity() {
        let a_kind = ProjectionKind::EqualEarth;
        let reference = a_kind.raw();
        let blend = BlendRaw::new(a_kind.raw(), ProjectionKind::Mercator.raw(), 0.0);
        let sim = anchoring_similarity(&blend, reference.as_ref(), 10.0, 45.0);
        assert!(
            sim.is_near_identity(),
            "expected identity correction, got {sim:?}"
        );
    }
}
