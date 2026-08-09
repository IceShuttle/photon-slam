/// Contains common boilerplate code
pub mod common;

/// Contains error types for shader
pub mod errors;
/// FAST-9 corner detection compute pass.
pub mod fast;
/// Gaussian blur compute pass.
pub mod gaussian_blur;
/// ORB intensity-centroid orientation compute pass.
pub mod orb;
/// YUVY-to-R8 luminance extraction compute pass.
pub mod yuvy_to_r8;
