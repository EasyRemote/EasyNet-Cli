// EasyNet RemoteApp — canonical wheel translation
// =================================================
//
// The browser data-channel contract carries CSS pixels. Platform adapters
// translate that single representation into their native continuous/detent
// units here so Windows and X11 cannot silently drift apart.

#[cfg(any(target_os = "windows", target_os = "linux", test))]
const PIXELS_PER_DETENT: f64 = 100.0;
#[cfg(any(target_os = "windows", test))]
const WINDOWS_UNITS_PER_DETENT: f64 = 120.0;
#[cfg(any(target_os = "windows", target_os = "linux", test))]
const MAX_DETENTS_PER_FRAME: f64 = 12.0;

#[cfg(any(target_os = "windows", test))]
pub(super) fn windows_native_units(pixels: Option<f64>) -> i32 {
    let pixels = pixels.unwrap_or(0.0);
    let native_delta = pixels * WINDOWS_UNITS_PER_DETENT / PIXELS_PER_DETENT;
    let max_delta = WINDOWS_UNITS_PER_DETENT * MAX_DETENTS_PER_FRAME;
    let bounded = native_delta.round().clamp(-max_delta, max_delta) as i32;
    if bounded == 0 && pixels != 0.0 {
        if pixels.is_sign_negative() {
            -1
        } else {
            1
        }
    } else {
        bounded
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn x11_detent_steps(pixels: Option<f64>) -> i32 {
    let Some(pixels) = pixels.filter(|value| *value != 0.0) else {
        return 0;
    };
    let steps = (pixels.abs() / PIXELS_PER_DETENT).ceil();
    let bounded = steps.clamp(1.0, MAX_DETENTS_PER_FRAME) as i32;
    if pixels.is_sign_negative() {
        -bounded
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests {
    use super::{windows_native_units, x11_detent_steps};

    #[test]
    fn windows_translation_preserves_high_resolution_delta_and_bounds_bursts() {
        assert_eq!(windows_native_units(Some(100.0)), 120);
        assert_eq!(windows_native_units(Some(-250.0)), -300);
        assert_eq!(windows_native_units(Some(0.1)), 1);
        assert_eq!(windows_native_units(Some(-0.1)), -1);
        assert_eq!(windows_native_units(Some(9_999.0)), 1_440);
        assert_eq!(windows_native_units(Some(-9_999.0)), -1_440);
        assert_eq!(windows_native_units(None), 0);
    }

    #[test]
    fn x11_translation_emits_bounded_discrete_detents() {
        assert_eq!(x11_detent_steps(Some(100.0)), 1);
        assert_eq!(x11_detent_steps(Some(-250.0)), -3);
        assert_eq!(x11_detent_steps(Some(0.1)), 1);
        assert_eq!(x11_detent_steps(Some(10_000.0)), 12);
        assert_eq!(x11_detent_steps(Some(-10_000.0)), -12);
        assert_eq!(x11_detent_steps(Some(0.0)), 0);
        assert_eq!(x11_detent_steps(None), 0);
    }
}
