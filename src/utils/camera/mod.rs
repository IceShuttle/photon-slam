use vulkano::format::Format;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "android")]
pub mod android;

/// Camera capture config
#[derive(Debug, Clone, Copy)]
pub struct CameraConfig {
    pub width: u32,
    pub height: u32,
    pub pixel_format: Format,
    pub hz: u32,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            pixel_format: Format::B8G8R8G8_422_UNORM,
            width: 640,
            height: 480,
            hz: 30,
        }
    }
}
