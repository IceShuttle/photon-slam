/// Captures frames from camera
pub mod camera;

/// Does tracing stuff
pub mod tracing;

#[macro_export]
macro_rules! compute_groups2D {
    ($extent:expr,$grp_size:expr) => {
        [
            $extent[0].div_ceil($grp_size),
            $extent[1].div_ceil($grp_size),
            1,
        ]
    };
}
