use chillffi::ffi::errors::FFIError;
use pyo3::PyErr;
use pyo3::exceptions::PyRuntimeError;
// =================================================================================================
// error.rs — перевод ошибок `chillffi` в ошибки Python.
//
// `toPyErr` — единственная точка конвертации `FFIError` → `PyRuntimeError`.
// =================================================================================================

// todo desc
pub fn toPyErr(error: FFIError) -> PyErr
{
  PyRuntimeError::new_err(format!("chillffi: {error:?}"))
}

// =================================================================================================
