use anyhow::Result;
use std::path::Path;

use super::model::{DiffMode, FileDiff};

pub trait DiffProvider {
    fn compute_diff(&self, repo_path: &Path, mode: DiffMode) -> Result<Vec<FileDiff>>;
}
