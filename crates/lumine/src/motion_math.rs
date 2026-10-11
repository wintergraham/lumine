use std::time::Duration;

/// Time curves. Cubic Bezier x controls must be in [0, 1]; y may overshoot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    CubicBezier(Bezier),
}

impl Easing {
    pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        assert!(
            [x1, y1, x2, y2].iter().all(|x| x.is_finite()),
            "non-finite easing"
        );
        assert!(
            (0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&x2),
            "Bezier x controls must be in [0, 1]"
        );
        Self::CubicBezier(Bezier {
            x: coefficients(x1, x2),
            y: coefficients(y1, y2),
        })
    }

    pub fn sample(self, t: f32) -> f32 {
        assert!(t.is_finite(), "non-finite easing time");
        let t = t.clamp(0.0, 1.0);
        if t == 0.0 || t == 1.0 {
            return t;
        }
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t,
            Self::EaseOut => t * (2.0 - t),
            Self::EaseInOut if t < 0.5 => 2.0 * t * t,
            Self::EaseInOut => 1.0 - 2.0 * (1.0 - t).powi(2),
            Self::CubicBezier(curve) => curve.sample(t),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bezier {
    x: [f64; 3],
    y: [f64; 3],
}

fn coefficients(a: f32, b: f32) -> [f64; 3] {
    let (a, b) = (a as f64, b as f64);
    [1.0 - 3.0 * b + 3.0 * a, 3.0 * b - 6.0 * a, 3.0 * a]
}
fn polynomial(c: [f64; 3], t: f64) -> f64 {
    ((c[0] * t + c[1]) * t + c[2]) * t
}
impl Bezier {
    fn sample(self, t: f32) -> f32 {
        let x = t as f64;
        let (mut lo, mut hi, mut s) = (0.0, 1.0, x);
        // Safeguarded Newton converges quickly on ordinary curves. The bracket
        // also handles zero derivatives and flat x controls without NaNs.
        for _ in 0..8 {
            let error = polynomial(self.x, s) - x;
            if error.abs() <= 1e-14 {
                return polynomial(self.y, s) as f32;
            }
            if error < 0.0 {
                lo = s;
            } else {
                hi = s;
            }
            let derivative = (3.0 * self.x[0] * s + 2.0 * self.x[1]) * s + self.x[2];
            let next = s - error / derivative;
            s = if next.is_finite() && next > lo && next < hi {
                next
            } else {
                (lo + hi) * 0.5
            };
        }
        for _ in 0..24 {
            let error = polynomial(self.x, s) - x;
            if error.abs() <= 1e-14 {
                break;
            }
            if error < 0.0 {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) * 0.5;
        }
        polynomial(self.y, s) as f32
    }
}

/// Physical spring settings. Rest thresholds use the animated value's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    pub rest_delta: f32,
    pub rest_speed: f32,
}
impl Default for Spring {
    fn default() -> Self {
        Self {
            stiffness: 320.0,
            damping: 30.0,
            mass: 1.0,
            rest_delta: 0.001,
            rest_speed: 0.01,
        }
    }
}
impl Spring {
    fn validate(self) {
        assert!(
            [
                self.stiffness,
                self.damping,
                self.mass,
                self.rest_delta,
                self.rest_speed
            ]
            .iter()
            .all(|x| x.is_finite() && *x > 0.0),
            "spring parameters must be finite and positive"
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimationKind {
    Tween { duration: Duration, easing: Easing },
    Spring(Spring),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    pub kind: AnimationKind,
    pub delay: Duration,
}
impl Transition {
    pub fn tween(duration: Duration, easing: Easing) -> Self {
        Self {
            kind: AnimationKind::Tween { duration, easing },
            delay: Duration::ZERO,
        }
    }
    pub fn spring(spring: Spring) -> Self {
        spring.validate();
        Self {
            kind: AnimationKind::Spring(spring),
            delay: Duration::ZERO,
        }
    }
    pub fn instant() -> Self {
        Self::tween(Duration::ZERO, Easing::Linear)
    }
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
    pub(crate) fn validate(self) {
        if let AnimationKind::Spring(spring) = self.kind {
            spring.validate();
        }
    }
}

/// Precomputed closed-form oscillator: no numerical integration or substeps.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Oscillator {
    Under {
        a: f64,
        b: f64,
        decay: f64,
        frequency: f64,
    },
    Critical {
        a: f64,
        b: f64,
        decay: f64,
    },
    Over {
        a: f64,
        b: f64,
        slow: f64,
        fast: f64,
    },
}
impl Oscillator {
    pub(crate) fn new(spring: Spring, displacement: f64, velocity: f64) -> Self {
        let omega = (spring.stiffness as f64 / spring.mass as f64).sqrt();
        let zeta =
            spring.damping as f64 / (2.0 * (spring.stiffness as f64 * spring.mass as f64).sqrt());
        let a = displacement;
        if (zeta - 1.0).abs() < 1e-6 {
            Self::Critical {
                a,
                b: velocity + omega * a,
                decay: omega,
            }
        } else if zeta < 1.0 {
            let decay = zeta * omega;
            let frequency = omega * (1.0 - zeta * zeta).sqrt();
            Self::Under {
                a,
                b: (velocity + decay * a) / frequency,
                decay,
                frequency,
            }
        } else {
            let sum = zeta + (zeta * zeta - 1.0).sqrt();
            let (slow, fast) = (-omega / sum, -omega * sum);
            let b = (velocity - slow * a) / (fast - slow);
            Self::Over {
                a: a - b,
                b,
                slow,
                fast,
            }
        }
    }
    pub(crate) fn sample(self, seconds: f64) -> (f64, f64) {
        let (x, v) = match self {
            Self::Under {
                a,
                b,
                decay,
                frequency,
            } => {
                let (sin, cos) = (frequency * seconds).sin_cos();
                let e = (-decay * seconds).exp();
                let x = a * cos + b * sin;
                (e * x, e * (frequency * (b * cos - a * sin) - decay * x))
            }
            Self::Critical { a, b, decay } => {
                let e = (-decay * seconds).exp();
                let x = a + b * seconds;
                (e * x, e * (b - decay * x))
            }
            Self::Over { a, b, slow, fast } => {
                let a = a * (slow * seconds).exp();
                let b = b * (fast * seconds).exp();
                (a + b, slow * a + fast * b)
            }
        };
        (x, v)
    }
}
