#[derive(Debug, thiserror::Error)]
pub enum ShaderDispatchError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error(transparent)]
    VulkanValidation(#[from] Box<vulkano::ValidationError>),
}
