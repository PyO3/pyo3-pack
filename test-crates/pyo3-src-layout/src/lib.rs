use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

#[pyfunction]
fn get_21() -> usize {
    21
}

#[pymodule]
fn pyo3_src_layout(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_wrapped(wrap_pyfunction!(get_21))?;

    Ok(())
}
