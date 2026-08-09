//! NoBS PDF's read-only inspection engine.

pub mod analysis;
pub mod app;
pub mod benchmark;
pub mod exporter;
pub mod flatten_pages;
pub mod geometry;
pub mod image_transformer;
pub mod model;
pub mod optimisation;
pub mod parser;
pub mod planner;
pub mod raster_merge;
pub mod rewriter;
pub mod validator;

use std::path::Path;

pub use model::AnalysisResult;
pub use parser::{InspectionError, LopdfParser, PdfParser};

/// Inspect a PDF without modifying it.
pub fn inspect(path: impl AsRef<Path>) -> Result<AnalysisResult, InspectionError> {
    LopdfParser.inspect(path.as_ref())
}

pub use planner::{create_plan, OptimisationPlan, PlannerConfig};
