use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct AnimationState {
    pub started_at: Instant,
    pub duration: Duration,
    pub hunk_index: usize,
}

impl AnimationState {
    pub fn new(hunk_index: usize) -> Self {
        Self {
            started_at: Instant::now(),
            duration: Duration::from_millis(150),
            hunk_index,
        }
    }

    /// Returns 0.0 to 1.0
    pub fn progress(&self) -> f64 {
        let elapsed = self.started_at.elapsed();
        (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0)
    }

    pub fn is_done(&self) -> bool {
        self.started_at.elapsed() >= self.duration
    }

    pub fn fade_in_opacity(&self) -> f64 {
        self.progress()
    }

    pub fn fade_out_opacity(&self) -> f64 {
        1.0 - self.progress()
    }

    pub fn opacity_to_brightness(opacity: f64) -> u8 {
        (opacity * 255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_animation_progress() {
        let anim = AnimationState::new(0);
        assert!(anim.progress() < 0.5);
        assert!(!anim.is_done());
    }

    #[test]
    fn test_animation_completes() {
        let mut anim = AnimationState::new(0);
        anim.duration = Duration::from_millis(10);
        thread::sleep(Duration::from_millis(20));
        assert!(anim.is_done());
        let progress = anim.progress();
        assert!(progress >= 0.99, "progress was {}", progress);
    }

    #[test]
    fn test_opacity_to_brightness() {
        assert_eq!(AnimationState::opacity_to_brightness(0.0), 0);
        assert_eq!(AnimationState::opacity_to_brightness(1.0), 255);
        assert_eq!(AnimationState::opacity_to_brightness(0.5), 127);
    }
}
