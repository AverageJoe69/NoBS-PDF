use std::{collections::HashMap, path::Path};

use lopdf::{Document, Object};
use thiserror::Error;

use crate::image_transformer::{resize_jpeg, TransformError};

#[derive(Debug, Clone)]
pub struct ApprovedReplacement {
    pub object_number: u32,
    pub generation: u16,
    pub source_pixels: [u32; 2],
    pub target_pixels: [u32; 2],
    pub colour_space: String,
}

#[derive(Debug, Error)]
pub enum RewriteError {
    #[error("PDF error: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("image {object_id} is no longer a stream")]
    MissingStream { object_id: String },
    #[error("image {object_id} transform failed: {source}")]
    Transform {
        object_id: String,
        source: TransformError,
    },
}

pub fn rewrite_pdf(
    input: &Path,
    temporary_output: &Path,
    replacements: &[ApprovedReplacement],
) -> Result<HashMap<String, u64>, RewriteError> {
    let mut document = Document::load(input)?;
    let mut encoded_sizes = HashMap::new();
    for replacement in replacements {
        let id = (replacement.object_number, replacement.generation);
        let object_id = format!("{} {} R", id.0, id.1);
        let stream = document.get_object_mut(id)?.as_stream_mut().map_err(|_| {
            RewriteError::MissingStream {
                object_id: object_id.clone(),
            }
        })?;
        let output = resize_jpeg(
            &stream.content,
            replacement.source_pixels,
            replacement.target_pixels,
            &replacement.colour_space,
        )
        .map_err(|source| RewriteError::Transform {
            object_id: object_id.clone(),
            source,
        })?;
        if output.len() >= stream.content.len() {
            continue;
        }
        stream.content = output;
        stream.dict.set(
            "Width",
            Object::Integer(i64::from(replacement.target_pixels[0])),
        );
        stream.dict.set(
            "Height",
            Object::Integer(i64::from(replacement.target_pixels[1])),
        );
        stream
            .dict
            .set("Length", Object::Integer(stream.content.len() as i64));
        encoded_sizes.insert(object_id, stream.content.len() as u64);
    }
    document.save(temporary_output)?;
    Ok(encoded_sizes)
}
