#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    XButton(u16),
    Key(u16),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}
impl Direction {
    pub fn index(self) -> usize {
        self as usize
    }
}
pub const MARKER: usize = 0x474c_4954_455f_3031;
pub fn is_own(extra: usize) -> bool {
    // Windows can return mouse extra-info with only its lower DWORD intact.
    // Accept precisely our full marker or its zero-extended DWORD form.
    extra == MARKER || extra == MARKER as u32 as usize
}
pub fn recognize(
    start: (i32, i32),
    now: (i32, i32),
    threshold: u32,
    ratio: f64,
) -> Option<Direction> {
    let dx = i64::from(now.0) - i64::from(start.0);
    let dy = i64::from(now.1) - i64::from(start.1);
    let x = dx.unsigned_abs() as f64;
    let y = dy.unsigned_abs() as f64;
    if x >= threshold as f64 && x >= y * ratio {
        Some(if dx > 0 {
            Direction::Right
        } else {
            Direction::Left
        })
    } else if y >= threshold as f64 && y >= x * ratio {
        Some(if dy > 0 {
            Direction::Down
        } else {
            Direction::Up
        })
    } else {
        None
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub enum Hold {
    #[default]
    Idle,
    Armed((i32, i32)),
    Consumed,
}
impl Hold {
    pub fn down(&mut self, point: (i32, i32)) {
        if matches!(self, Self::Idle) {
            *self = Self::Armed(point);
        }
    }
    pub fn movement(&mut self, point: (i32, i32), threshold: u32, ratio: f64) -> Option<Direction> {
        if let Self::Armed(start) = *self {
            if let Some(d) = recognize(start, point, threshold, ratio) {
                *self = Self::Consumed;
                return Some(d);
            }
        }
        None
    }
    pub fn release(&mut self, passthrough: bool) -> bool {
        let tap = passthrough && matches!(self, Self::Armed(_));
        *self = Self::Idle;
        tap
    }
    pub fn active(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cardinal_thresholds() {
        for (p, d) in [
            ((-18, 0), Direction::Left),
            ((18, 0), Direction::Right),
            ((0, -18), Direction::Up),
            ((0, 18), Direction::Down),
        ] {
            assert_eq!(recognize((0, 0), p, 18, 1.25), Some(d));
        }
        assert_eq!(recognize((0, 0), (17, 0), 18, 1.25), None);
    }
    #[test]
    fn dominance() {
        assert_eq!(recognize((0, 0), (20, 20), 18, 1.25), None);
        assert_eq!(
            recognize((0, 0), (25, 20), 18, 1.25),
            Some(Direction::Right)
        );
        assert_eq!(recognize((0, 0), (24, 20), 18, 1.25), None);
    }
    #[test]
    fn signed_extremes() {
        assert_eq!(
            recognize((i32::MIN, 0), (i32::MAX, 0), 18, 1.25),
            Some(Direction::Right)
        );
        assert_eq!(
            recognize((-100, -100), (-120, -100), 18, 1.25),
            Some(Direction::Left)
        );
    }
    #[test]
    fn once_per_hold_and_rearm() {
        let mut h = Hold::Idle;
        h.down((0, 0));
        h.down((10, 0));
        assert_eq!(h.movement((18, 0), 18, 1.25), Some(Direction::Right));
        assert_eq!(h.movement((-30, 0), 18, 1.25), None);
        assert!(!h.release(true));
        h.down((0, 0));
        assert_eq!(h.movement((0, -18), 18, 1.25), Some(Direction::Up));
    }
    #[test]
    fn tap_and_reset() {
        let mut h = Hold::Idle;
        assert!(!h.release(true));
        h.down((0, 0));
        assert!(h.release(true));
        h.down((0, 0));
        assert!(!h.release(false));
        h.down((0, 0));
        h = Hold::Idle;
        assert!(!h.active());
        assert_eq!(h.movement((100, 0), 18, 1.25), None);
    }
    #[test]
    fn own_marker_only() {
        assert!(is_own(MARKER));
        assert!(!is_own(0));
        assert!(!is_own(1234));
    }
    #[test]
    fn mouse_marker_survives_observed_dword_truncation() {
        assert_eq!(MARKER as u32 as usize, 0x455f_3031);
        assert!(is_own(0x455f_3031));
        assert!(!is_own(0x1111_1111_455f_3031));
        assert!(!is_own(0x455f_3030));
    }
}
