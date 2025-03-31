use pyo3::prelude::*;
use pythonize::pythonize;

#[pyfunction]
fn parse_file(py: Python, path: String) -> PyResult<pyo3::Bound<'_, PyAny>> {
    let file = std::fs::File::open(&path)?;
    let value = ::ziplinter::parse_file(&file);

    Ok(pythonize(py, &value)?)
}

#[pyfunction]
fn parse_bytes(py: Python, bytes: Vec<u8>) -> PyResult<pyo3::Bound<'_, PyAny>> {
    let value = ::ziplinter::parse_bytes(&bytes);
    Ok(pythonize(py, &value)?)
}

#[pymodule]
fn ziplinter(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_file, m)?)?;
    m.add_function(wrap_pyfunction!(parse_bytes, m)?)?;

    Ok(())
}
