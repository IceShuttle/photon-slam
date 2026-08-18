/// Captures frames from camera
pub mod camera;

/// Does tracing stuff
pub mod tracing;

pub const fn compute_groups_2d(extent: [u32; 3], group_size: u32) -> [u32; 3] {
    [
        extent[0].div_ceil(group_size),
        extent[1].div_ceil(group_size),
        1,
    ]
}
