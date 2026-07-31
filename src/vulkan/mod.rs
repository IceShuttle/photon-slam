/// Holds instance, device, queue, and allocators.
pub mod context;
/// Handles things related to display
pub mod disp;
/// Contains shader helper structs to dispatch them through a command buffer
///
/// Feature extraction compute passes (FAST, ORB, …)
pub mod shaders;
/// Handles things regarding Initialization
pub mod system;
