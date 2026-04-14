use crate::ml::embedding::EMBEDDING_DIM;

pub const MAX_ANTI_TARGETS: usize = 8;

#[derive(Debug, Clone)]
pub struct AntiTarget {
    pub name: String,
    pub embedding: Vec<f32>,
}

impl AntiTarget {
    pub fn new(name: String, embedding: Vec<f32>) -> Self {
        debug_assert_eq!(embedding.len(), EMBEDDING_DIM);
        Self { name, embedding }
    }
}
