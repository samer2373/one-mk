//! TODO: native backend.

use crate::Backend;
use anyhow::Result;
use std::path::PathBuf;

pub(crate) async fn open(_state_dir: PathBuf) -> Result<Backend> {
    anyhow::bail!("not implemented yet")
}
