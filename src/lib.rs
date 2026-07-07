//! Rust core for `simplelocalize-download`.
//!
//! The compiled extension module is exposed to Python as
//! `simplelocalize_download._core`. Feature work (download engine, bindings)
//! lands in later modules; this crate currently provides the API client and the
//! module boundary.

pub mod api;
pub mod download;
pub mod error;
pub mod model;
pub mod python;
pub mod retry;
pub mod template;

use pyo3::prelude::*;

/// The `_core` extension module.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    python::register(m)?;
    Ok(())
}
