use crate::error::toPyErr;
use crate::scope::ScopeSlot;
use chillffi::ffi::scope::{FFIScope, Scope as ChillScope};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyType};
use std::sync::Arc;
// =================================================================================================
// memory.rs — блок памяти в куче клона.
//
// `Memory` — проброс `AllocatedMemory` + `Scope::readMemory`/`writeMemory`:
// данные живут в клоне, хост читает/пишет их через IPC. Освобождение — RAII.
// =================================================================================================

/// Блок памяти в куче клона — проброс `AllocatedMemory` + `Scope::readMemory`
/// / `Scope::writeMemory`. Данные живут в клоне: C видит их по адресу
/// `asPointer()`, хост читает/пишет через IPC. Освобождение — `Scope::free`
/// при сборке объекта или закрытии скоупа.
///
/// Статические структуры — `readStruct(cls)` / `writeStruct(inst)`:
/// `ctypes.Structure` здесь — то, чем в Rust был `#[repr(C)]` + Pod.
/// Динамические формы — `scope.allocStruct`/`readDynamicStruct`/
/// `writeDynamicStruct` (`scope.rs`); по значению — `chillffi.StructValue`.
#[pyclass(name = "Memory")]
pub struct PyMemory
{
  // todo desc
  pub slot: Arc<ScopeSlot>,

  // todo desc
  pub address: usize,

  // todo desc
  pub length: usize
}

#[pymethods]
impl PyMemory
{
  /// Размер блока в байтах.
  fn length(&self) -> usize
  {
    self.length
  }

  /// Адрес блока в пространстве клона (не хоста!).
  fn address(&self) -> usize
  {
    self.address
  }

  /// Адрес как аргумент — то же, что положить сам объект в `.arg(...)`.
  fn asPointer(&self) -> usize
  {
    self.address
  }

  /// `AllocatedMemory::write`: пишет байты в клон (`Scope::writeMemory`).
  fn write(&self, py: Python<'_>, data: Vec<u8>) -> PyResult<()>
  {
    self.checkOpen()?;
    let address: usize = self.address;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::writeMemory(address, data)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)
  }

  /// `AllocatedMemory::read`: читает весь блок из клона (`Scope::readMemory`).
  fn read(&self, py: Python<'_>) -> PyResult<Py<PyAny>>
  {
    self.checkOpen()?;
    let address: usize = self.address;
    let length: usize = self.length;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    let bytes: Vec<u8> = py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::readMemory(address, length)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)?;
    Ok(PyBytes::new(py, &bytes).into_any().unbind())
  }

  /// `AllocatedMemory::readStruct::<T>()` — читает блок как статически
  /// типизированную C-структуру. `T` в Rust — `#[repr(C)]` + Pod (bytemuck);
  /// здесь — подкласс `ctypes.Structure` (`_fields_`): раскладку считает
  /// ctypes по тем же C-правилам, `from_buffer_copy` — аналог
  /// `pod_read_unaligned` (переинтерпретация байтов).
  ///
  /// Читаются только байты, которые нужны типу (`ctypes.sizeof(cls)`) —
  /// блок может быть больше; размер проверяется как в ядре.
  ///
  /// ```python
  /// class Clock(ctypes.Structure):
  ///     _fields_ = [("tv_sec", ctypes.c_int64), ("tv_nsec", ctypes.c_long)]
  /// clock = mem.readStruct(Clock)
  /// clock.tv_sec
  /// ```
  fn readStruct<'py>(&self, py: Python<'py>, cls: &Bound<'py, PyType>) -> PyResult<Bound<'py, PyAny>>
  {
    self.checkOpen()?;
    let ctypes: Bound<'py, PyAny> = py.import("ctypes")?.into_any();
    let structure: Bound<'py, PyType> = ctypes.getattr("Structure")?.cast_into()?;
    if !cls.is_subclass(&structure)?
    {
      return Err(PyRuntimeError::new_err(format!(
        "readStruct: {} is not a ctypes.Structure subclass — the static layout \
         comes from `_fields_`, as `#[repr(C)]` + Pod in Rust",
        cls.name()?
      )));
    }

    let expectedSize: usize = ctypes.call_method1("sizeof", (cls,))?.extract()?;
    if self.length < expectedSize
    {
      return Err(PyRuntimeError::new_err(format!(
        "readStruct: buffer is {} bytes, but {} is {} bytes",
        self.length, cls.name()?, expectedSize
      )));
    }

    // Читаем только байты, нужные типу — не весь блок (как readStruct в ядре).
    // Чтение идёт на потоке-владельце клона (ScopeSlot::withScope).
    let address: usize = self.address;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    let bytes: Vec<u8> = py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::readMemory(address, expectedSize)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)?;
    if bytes.len() < expectedSize
    {
      return Err(PyRuntimeError::new_err(format!(
        "readStruct: received {} bytes, expected at least {}",
        bytes.len(), expectedSize
      )));
    }

    // pod_read_unaligned в Rust; ctypes на своей платформе раскладывает так же.
    cls.call_method1("from_buffer_copy", (PyBytes::new(py, &bytes),))
  }

  /// `AllocatedMemory::writeStruct::<T>(&value)` — пишет статически
  /// типизированную C-структуру (`ctypes.Structure`) в блок.
  ///
  /// ```python
  /// point = Point(10, 32)  # class Point(ctypes.Structure): _fields_ = ...
  /// mem.writeStruct(point)
  /// ```
  fn writeStruct(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()>
  {
    self.checkOpen()?;
    let ctypes: Bound<'_, PyAny> = py.import("ctypes")?.into_any();
    let structure: Bound<'_, PyType> = ctypes.getattr("Structure")?.cast_into()?;
    if !value.is_instance(&structure)?
    {
      return Err(PyRuntimeError::new_err(format!(
        "writeStruct: {} is not a ctypes.Structure instance — \
         динамические формы — scope.writeDynamicStruct, структуры по значению — chillffi.StructValue",
        value.get_type().name()?
      )));
    }

    let cls: Bound<'_, PyType> = value.get_type();
    let expectedSize: usize = ctypes.call_method1("sizeof", (&cls,))?.extract()?;
    if self.length < expectedSize
    {
      return Err(PyRuntimeError::new_err(format!(
        "writeStruct: buffer is {} bytes, but {} is {} bytes",
        self.length, cls.name()?, expectedSize
      )));
    }

    // Байты инстанса (buffer protocol) — как bytemuck::bytes_of в Rust.
    let bytes: Bound<'_, PyAny> = py.import("builtins")?.getattr("bytes")?.call1((value,))?;
    let data: Vec<u8> = bytes.extract()?;
    let address: usize = self.address;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::writeMemory(address, data)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)
  }

  // todo desc
  fn __repr__(&self) -> String
  {
    format!("<Memory address={:#x} length={}>", self.address, self.length)
  }
}

impl PyMemory
{
  // todo desc
  fn checkOpen(&self) -> PyResult<()>
  {
    if self.slot.isOpen() {
      Ok(())
    } else {
      Err(PyRuntimeError::new_err("scope is closed"))
    }
  }
}

impl Drop for PyMemory
{
  /// RAII: блок освобождается `Scope::free`, пока скоуп жив.
  fn drop(&mut self) -> ()
  {
    if self.slot.isOpen()
    {
      let address: usize = self.address;
      let _ = self.slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::free(address)
      });
    }
  }
}

// =================================================================================================
