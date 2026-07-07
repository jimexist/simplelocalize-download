//! Rust core for `simplelocalize-download`.
//!
//! The compiled extension module is exposed to Python as
//! `simplelocalize_download._core`. Feature work (API client, download engine,
//! bindings) lands in later modules; this scaffold only establishes the module
//! boundary and surfaces the crate version.

use pyo3::prelude::*;

/// The `_core` extension module.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
