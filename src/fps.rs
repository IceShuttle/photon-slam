use std::time::Instant;

/// Interval at which FPS is logged.
const LOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Tracks frame timing and prints FPS via `tracing::info!`.
///
/// Call [`tick`](Self::tick) once per frame. Every `LOG_INTERVAL` (1 s) it
/// emits a structured tracing event with the average FPS over the window.
pub struct FpsCounter {
    frame_count: u64,
    last_log: Instant,
}

impl FpsCounter {
    pub fn new() -> Self {
        Self {
            frame_count: 0,
            last_log: Instant::now(),
        }
    }

    /// Advance one frame. Logs FPS when the interval elapses and resets.
    pub fn tick(&mut self) {
        self.frame_count += 1;
        let elapsed = self.last_log.elapsed();
        if elapsed >= LOG_INTERVAL {
            let fps = self.frame_count as f64 / elapsed.as_secs_f64();
            tracing::debug!(
                fps = fps,
                frames = self.frame_count,
                elapsed_ms = elapsed.as_millis() as u64,
                "FPS",
            );
            self.frame_count = 0;
            self.last_log = Instant::now();
        }
    }
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self::new()
    }
}
