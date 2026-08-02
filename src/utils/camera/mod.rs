#[cfg(target_os = "linux")]
pub mod linux;

pub mod android;

/// Camera capture config
#[derive(Debug, Clone, Copy)]
pub struct CameraConfig {
    pub width: u32,
    pub height: u32,
    pub pixel_format: v4l::FourCC,
    pub hz: u32,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            pixel_format: v4l::FourCC::new(b"YUYV"),
            width: 640,
            height: 480,
            hz: 30,
        }
    }
}
