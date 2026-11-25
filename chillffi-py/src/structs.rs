use crate::args::{FfiArgValue, pyToFfiArg, valueToArg};
use chillffi::ffi::types::Type as FfiType;
use chillffi::ffi::types::primitive::{Pointer as FfiPointer, StructValue as FfiStructValue};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyBytes, PyType};
// =================================================================================================
// structs.rs — движок структур: `Type` и `StructValue`
// (зеркало `chillffi/src/ffi/types`).
//
// Три уровня структур ядра, все проброшены:
//
//  * форма раскладки — `chillffi.Type`: скаляры, `Pointer` и вложенные
//    `Type.Structure([...])`;
//  * динамические — `scope.allocStruct` / `readDynamicStruct` /
//    `writeDynamicStruct` (`scope.rs`): раскладку считает клон — та же
//    ABI-математика `libffi`, что и у вызовов;
//  * по значению — `chillffi.StructValue([...])` в `.arg()` и
//    `.resultStruct([...])` на билдере (`library.rs`).
// =================================================================================================
// Type (python): зеркало `chillffi::ffi::types::Type`

/// Форма раскладки структуры — зеркало `chillffi::ffi::types::Type`.
///
/// Скаляры — константы (`chillffi.Type.I32`, `chillffi.Type.Pointer`, ...),
/// вложенные структуры — `chillffi.Type.Structure([...])` (аналог
/// `Type::structure([...])` в Rust). Форма нужна движку структур:
/// `scope.allocStruct([...])`, `scope.readDynamicStruct(addr, [...])`,
/// `scope.writeDynamicStruct(...)`, `.resultStruct([...])`.
#[pyclass(name = "Type")]
pub struct PyFfiType
{
  // todo desc
  pub(crate) t: FfiType
}

#[pymethods]
impl PyFfiType
{
  // ===============================================================================================
  
  /// `Type::I8` — скалярное поле раскладки.
  #[classattr]
  pub fn I8() -> Self
  {
    PyFfiType { t: FfiType::I8 }
  }
  
  /// `Type::I16` — скалярное поле раскладки.
  #[classattr]
  pub fn I16() -> Self
  {
    PyFfiType { t: FfiType::I16 }
  }
  
  /// `Type::I32` — скалярное поле раскладки.
  #[classattr]
  pub fn I32() -> Self
  {
    PyFfiType { t: FfiType::I32 }
  }
  
  /// `Type::I64` — скалярное поле раскладки.
  #[classattr]
  pub fn I64() -> Self
  {
    PyFfiType { t: FfiType::I64 }
  }
  
  /// `Type::Isize` — скалярное поле раскладки.
  #[classattr]
  pub fn Isize() -> Self
  {
    PyFfiType { t: FfiType::Isize }
  }
  
  /// `Type::U8` — скалярное поле раскладки.
  #[classattr]
  pub fn U8() -> Self
  {
    PyFfiType { t: FfiType::U8 }
  }
  
  /// `Type::U16` — скалярное поле раскладки.
  #[classattr]
  pub fn U16() -> Self
  {
    PyFfiType { t: FfiType::U16 }
  }
  
  /// `Type::U32` — скалярное поле раскладки.
  #[classattr]
  pub fn U32() -> Self
  {
    PyFfiType { t: FfiType::U32 }
  }
  
  /// `Type::U64` — скалярное поле раскладки.
  #[classattr]
  pub fn U64() -> Self
  {
    PyFfiType { t: FfiType::U64 }
  }
  
  /// `Type::Usize` — скалярное поле раскладки.
  #[classattr]
  pub fn Usize() -> Self
  {
    PyFfiType { t: FfiType::Usize }
  }
  
  /// `Type::F32` — скалярное поле раскладки.
  #[classattr]
  pub fn F32() -> Self
  {
    PyFfiType { t: FfiType::F32 }
  }
  
  /// `Type::F64` — скалярное поле раскладки.
  #[classattr]
  pub fn F64() -> Self {
    PyFfiType { t: FfiType::F64 }
  }
  
  /// `Type::Bool` — скалярное поле раскладки.
  #[classattr]
  pub fn Bool() -> Self
  {
    PyFfiType { t: FfiType::Bool }
  }

  // ===============================================================================================
  
  /// `Type::Pointer` — поле-указатель. Значение поля — адрес в пространстве
  /// клона (`Memory.address()`, адрес от C); адрес хоста в клоне невалиден.
  #[classattr]
  pub fn Pointer() -> Self
  {
    PyFfiType { t: FfiType::Pointer }
  }

  /// `Type::structure(fields)` — вложенная структура как поле.
  ///
  /// ```python
  /// shape = [chillffi.Type.Structure([chillffi.Type.I32, chillffi.Type.I32]), chillffi.Type.F32]
  /// mem = scope.allocStruct(shape)
  /// ```
  #[classmethod]
  pub fn Structure(_cls: &Bound<'_, PyType>, fields: &Bound<'_, PyAny>) -> PyResult<Self>
  {
    let nested: Vec<FfiType> = shapeFromPy(fields)?;
    Ok(PyFfiType { t: FfiType::Struct(nested.into_boxed_slice()) })
  }

  // todo desc
  fn __repr__(&self) -> String
  {
    typeRepr(&self.t)
  }

  /// Сравнение форм (ядро не даёт `PartialEq` для `Type` наружу — сравниваем
  /// канонический Debug).
  fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool>
  {
    match other.cast::<PyFfiType>()
    {
      Ok(o) => Ok(format!("{:?}", self.t) == format!("{:?}", o.borrow().t)),
      Err(_) => Ok(false)
    }
  }

  // ===============================================================================================
}

// =================================================================================================

/// Рекурсивный repr: `chillffi.Type.Structure([chillffi.Type.I32, ...])`.
fn typeRepr(t: &FfiType) -> String
{
  match t
  {
    FfiType::Struct(fields) => format!(
      "chillffi.Type.Structure([{}])",
      fields.iter().map(typeRepr).collect::<Vec<_>>().join(", ")
    ),
    other => format!("chillffi.Type.{other:?}")
  }
}

/// Python-список `chillffi.Type` → `Vec<Type>` ядра. Общий вход для
/// `allocStruct`/`readDynamicStruct`/`writeDynamicStruct`/`.resultStruct`.
pub(crate) fn shapeFromPy(object: &Bound<'_, PyAny>) -> PyResult<Vec<FfiType>>
{
  let mut shape: Vec<FfiType> = Vec::new();
  for item in object.try_iter()?
  {
    let item = item?;
    let t: PyRef<'_, PyFfiType> = match item.extract()
    {
      Ok(t) => t,
      Err(_) =>
      {
        let name: String = item.get_type().name()
          .map(|n| n.to_string_lossy().into_owned())
          .unwrap_or_else(|_| "?".to_string());
        return Err(PyRuntimeError::new_err(format!(
          "ожидаю список chillffi.Type (chillffi.Type.I32, chillffi.Type.Structure([...])), получил {name}"
        )));
      }
    };
    shape.push(t.t.clone());
  }
  Ok(shape)
}

// =================================================================================================
// StructValue (python): DynamicList/StructValue ядра — один класс

/// Структура: по значению в `.arg()` и результат `.resultStruct([...])` /
/// `scope.readDynamicStruct(...)` — один класс, как в ядре (`DynamicList` и
/// `StructValue` там — одна representation).
///
/// Построение — список полей, типы inferred из значений (`StructValue::new`
/// в Rust: `Arg::from(10i32)` здесь `chillffi.I32(10)` — голый int ушёл бы
/// как i64 и сломал бы ABI-классификацию):
///
/// ```python
/// point = chillffi.StructValue([chillffi.I32(10), chillffi.I32(32)])
/// lib.pointSum.arg(point).result("i32")
/// ```
///
/// Поле-указатель — `chillffi.Pointer(...)` с адресом клона (`Memory.address()`
/// или адрес, вернувшийся от C). Строка-поле — `chillffi.CString(...)`/
/// `bytes` (один указатель); голый `str` (ptr+len, два C-аргумента) полем
/// быть не может — как в Rust (`Value::String cannot be a field`).
///
/// Чтение — по индексу: `out.get(0)`, вложенное — `out.getStruct(0)`,
/// размер — `len(out)` / `out.isEmpty()`.
#[pyclass(name = "StructValue")]
pub struct PyStructValue
{
  /// Поля в FFI-представлении. Есть всегда: у построенного — из значений,
  /// у полученного — восстановлены по форме (чтобы структуру можно было
  /// передать дальше в `.arg()` с правильными ширинами).
  pub(crate) fields: Vec<FfiArgValue>,

  /// Готовые Python-значения — только у полученного
  /// (`readDynamicStruct`/`.resultStruct`). `get()`/`getStruct()` читают их.
  values: Option<Vec<Py<PyAny>>>
}

#[pymethods]
impl PyStructValue
{
  // ===============================================================================================
  
  /// Построение: список полей, типы inferred из значений.
  /// Вложенные структуры — вложенные `chillffi.StructValue([...])`.
  #[new]
  fn new(fields: &Bound<'_, PyAny>) -> PyResult<Self>
  {
    let mut parsed: Vec<FfiArgValue> = Vec::new();
    for item in fields.try_iter()?
    {
      parsed.push(pyToFfiArg(item?)?);
    }
    Ok(PyStructValue { fields: parsed, values: None })
  }

  /// Число полей (протокол `len(out)`; у ядра — `len()`).
  fn __len__(&self) -> usize
  {
    self.fields.len()
  }

  /// Пуста ли структура.
  fn isEmpty(&self) -> bool
  {
    self.fields.is_empty()
  }

  /// Поле по индексу (прочитанное значение: int/float/bool/адрес/StructValue).
  fn get(&self, py: Python<'_>, index: isize) -> PyResult<Py<PyAny>>
  {
    let values: &Vec<Py<PyAny>> = self.received()?;
    match values.get(indexMax(index, values.len())?)
    {
      Some(value) => Ok(value.clone_ref(py)),
      None => Err(boundError(index, values.len()))
    }
  }

  /// Вложенное поле-структура по индексу (`getStruct` ядра).
  fn getStruct(&self, py: Python<'_>, index: isize) -> PyResult<Py<PyStructValue>>
  {
    let values: &Vec<Py<PyAny>> = self.received()?;
    let i: usize = indexMax(index, values.len())?;
    match values.get(i)
    {
      Some(value) => match value.bind(py).clone().cast_into::<PyStructValue>()
      {
        Ok(structValue) => Ok(structValue.unbind()),
        Err(_) => Err(PyRuntimeError::new_err(format!(
          "field {index}: expected Struct, got {}",
          value.bind(py).get_type().name()?
        )))
      },
      None => Err(boundError(index, values.len()))
    }
  }

  // todo desc
  fn __repr__(&self, py: Python<'_>) -> String
  {
    match &self.values
    {
      // Прочитанная: значения как есть.
      Some(values) =>
      {
        let items: Vec<String> = values.iter()
          .map(|v| v.bind(py).repr().map(|r| r.to_string_lossy().into_owned()).unwrap_or_else(|_| "<unrepr>".to_string()))
          .collect();
        format!("chillffi.StructValue([{}])", items.join(", "))
      }
      // Построенная: FFI-поля.
      None => format!("chillffi.StructValue({:?})", self.fields)
    }
  }

  // ===============================================================================================

  /// Полученная структура (для `get`/`getStruct`).
  fn received(&self) -> PyResult<&Vec<Py<PyAny>>>
  {
    self.values.as_ref().ok_or_else(|| PyRuntimeError::new_err(
      "this StructValue was built in Python (not read from a call): it has no \
       readable values yet — get()/getStruct() work on the result of \
       scope.readDynamicStruct() / .resultStruct()"
    ))
  }

  // ===============================================================================================
}

// =================================================================================================

/// Индекс с проверкой границ (отрицательные — как в ядре: out of bounds).
fn indexMax(index: isize, len: usize) -> PyResult<usize>
{
  if index < 0 || index as usize >= len
  {
    return Err(boundError(index, len));
  }
  Ok(index as usize)
}

// todo desc
fn boundError(index: isize, len: usize) -> PyErr
{
  PyRuntimeError::new_err(format!("field index {index} out of bounds (len {len})"))
}

// =================================================================================================
// Ядро → Python

/// `StructValue`/`DynamicList` ядра → `PyStructValue` по форме, под которой
/// клон читал поля. Каждое поле извлекается тем типом, каким его прочитали:
/// `get::<i64>`, `get::<bool>`, `get::<Pointer>`, вложенное — `getStruct`.
/// Одновременно восстанавливаются FFI-поля (передать обратно в `.arg()`).
pub(crate) fn structValueToPython(
  py: Python<'_>,
  structValue: &FfiStructValue,
  shape: &[FfiType]
) -> PyResult<Py<PyAny>>
{
  let fields: Vec<FfiArgValue> = structFields(structValue, shape)?;
  let mut values: Vec<Py<PyAny>> = Vec::with_capacity(shape.len());

  for (field, index) in shape.iter().zip(0..)
  {
    values.push(match field
    {
      FfiType::Bool => PyBool::new(py, structValue.get::<bool>(index).map_err(toTypeErr)?)
        .to_owned().into_any().unbind(),

      FfiType::F32 =>
      {
        let v: f32 = structValue.get::<f32>(index).map_err(toTypeErr)?;
        f64::from(v).into_pyobject(py)?.into_any().unbind()
      }
      FfiType::F64 =>
      {
        let v: f64 = structValue.get::<f64>(index).map_err(toTypeErr)?;
        v.into_pyobject(py)?.into_any().unbind()
      }

      // Адрес (Pointer) — как `.result("pointer")`: наружу Python int.
      FfiType::Pointer =>
      {
        let v: FfiPointer = structValue.get::<FfiPointer>(index).map_err(toTypeErr)?;
        v.0.into_pyobject(py)?.into_any().unbind()
      }

      // Вложенная структура — рекурсивно по вложенной форме.
      FfiType::Struct(nested) =>
      {
        let inner: FfiStructValue = structValue.getStruct(index).map_err(toTypeErr)?;
        structValueToPython(py, &inner, nested)?
      }

      // Целые: знак по семантике типа (i64 → Int, u64 → UInt), как `dispatch`.
      FfiType::I8 =>
      {
        let v: i8 = structValue.get::<i8>(index).map_err(toTypeErr)?;
        i64::from(v).into_pyobject(py)?.into_any().unbind()
      }
      FfiType::I16 =>
      {
        let v: i16 = structValue.get::<i16>(index).map_err(toTypeErr)?;
        i64::from(v).into_pyobject(py)?.into_any().unbind()
      }
      FfiType::I32 =>
      {
        let v: i32 = structValue.get::<i32>(index).map_err(toTypeErr)?;
        i64::from(v).into_pyobject(py)?.into_any().unbind()
      }
      FfiType::I64 =>
      {
        let v: i64 = structValue.get::<i64>(index).map_err(toTypeErr)?;
        v.into_pyobject(py)?.into_any().unbind()
      }
      FfiType::Isize =>
      {
        let v: isize = structValue.get::<isize>(index).map_err(toTypeErr)?;
        (v as i64).into_pyobject(py)?.into_any().unbind()
      }
      FfiType::U8 =>
      {
        let v: u8 = structValue.get::<u8>(index).map_err(toTypeErr)?;
        u64::from(v).into_pyobject(py)?.into_any().unbind()
      }
      FfiType::U16 =>
      {
        let v: u16 = structValue.get::<u16>(index).map_err(toTypeErr)?;
        u64::from(v).into_pyobject(py)?.into_any().unbind()
      }
      FfiType::U32 =>
      {
        let v: u32 = structValue.get::<u32>(index).map_err(toTypeErr)?;
        u64::from(v).into_pyobject(py)?.into_any().unbind()
      }
      FfiType::U64 =>
      {
        let v: u64 = structValue.get::<u64>(index).map_err(toTypeErr)?;
        v.into_pyobject(py)?.into_any().unbind()
      }
      FfiType::Usize =>
      {
        let v: usize = structValue.get::<usize>(index).map_err(toTypeErr)?;
        (v as u64).into_pyobject(py)?.into_any().unbind()
      }

      FfiType::None => PyBytes::new(py, &[]).into_any().unbind()
    });
  }

  let received: PyStructValue = PyStructValue { fields, values: Some(values) };
  Ok(Py::new(py, received)?.into_any())
}

// todo desc
fn toTypeErr(error: chillffi::ffi::errors::FFIError) -> PyErr
{
  PyRuntimeError::new_err(format!("chillffi: {error:?}"))
}

/// FFI-поля прочитанной структуры, восстановленные по форме (ширины — те,
/// под которыми клон читал): `.arg()` примет её обратно без искажений.
fn structFields(structValue: &FfiStructValue, shape: &[FfiType]) -> PyResult<Vec<FfiArgValue>>
{
  let mut fields: Vec<FfiArgValue> = Vec::with_capacity(shape.len());
  for (field, index) in shape.iter().zip(0..)
  {
    fields.push(match field
    {
      FfiType::Bool => FfiArgValue::Bool(structValue.get::<bool>(index).map_err(toTypeErr)?),
      FfiType::F32 => FfiArgValue::F32(structValue.get::<f32>(index).map_err(toTypeErr)?),
      FfiType::F64 => FfiArgValue::F64(structValue.get::<f64>(index).map_err(toTypeErr)?),
      FfiType::Pointer => FfiArgValue::Pointer(structValue.get::<FfiPointer>(index).map_err(toTypeErr)?.0),
      FfiType::Struct(nested) =>
      {
        let inner: FfiStructValue = structValue.getStruct(index).map_err(toTypeErr)?;
        FfiArgValue::Struct(structFields(&inner, nested)?)
      }
      FfiType::I8    => FfiArgValue::I8(structValue.get::<i8>(index).map_err(toTypeErr)?),
      FfiType::I16   => FfiArgValue::I16(structValue.get::<i16>(index).map_err(toTypeErr)?),
      FfiType::I32   => FfiArgValue::I32(structValue.get::<i32>(index).map_err(toTypeErr)?),
      FfiType::I64   => FfiArgValue::I64(structValue.get::<i64>(index).map_err(toTypeErr)?),
      FfiType::Isize => FfiArgValue::Isize(structValue.get::<isize>(index).map_err(toTypeErr)?),
      FfiType::U8    => FfiArgValue::U8(structValue.get::<u8>(index).map_err(toTypeErr)?),
      FfiType::U16   => FfiArgValue::U16(structValue.get::<u16>(index).map_err(toTypeErr)?),
      FfiType::U32   => FfiArgValue::U32(structValue.get::<u32>(index).map_err(toTypeErr)?),
      FfiType::U64   => FfiArgValue::U64(structValue.get::<u64>(index).map_err(toTypeErr)?),
      FfiType::Usize => FfiArgValue::Usize(structValue.get::<usize>(index).map_err(toTypeErr)?),
      FfiType::None  => return Err(PyRuntimeError::new_err(
        "Type.None cannot be a struct field"))
    });
  }
  Ok(fields)
}

/// Построенная в Python структура → `Vec<Arg>` ядра (для
/// `Scope::writeDynamicStruct` — значения полей там идут списком `Arg`).
pub(crate) fn argsFromPy(values: &Bound<'_, PyAny>) -> PyResult<Vec<chillffi::ffi::types::primitive::Arg>>
{
  let mut args: Vec<chillffi::ffi::types::primitive::Arg> = Vec::new();
  for item in values.try_iter()?
  {
    args.push(valueToArg(&pyToFfiArg(item?)?));
  }
  Ok(args)
}

// =================================================================================================
