use crate::callback::PyCallback;
use crate::memory::PyMemory;
use crate::pytypes::{ctypesArg, typedArg};
use chillffi::ffi::library::{CallBuilder, VariadicCallBuilder};
use chillffi::ffi::types::primitive::{Arg, Callback as FfiCallback, Pointer as FfiPointer, StructValue as FfiStructValue};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyAny;
// =================================================================================================
// args.rs — типизированные аргументы вызова.
//
// `FfiArgValue` — Python-объект → типизированный C-аргумент (`pyToFfiArg`),
// приклеивание к билдеру — `pushArg` / `pushVariadicArg`.
// =================================================================================================

/// Конкретный типизированный аргумент, который можно
/// положить в `CallBuilder::arg::<T>`. Конвертируется из Python-объекта
/// под GIL, до отпускания GIL.
///
/// Важно про семантику строк в `chillffi` (см. `worker.rs` и `types/primitive/strings.rs`):
///  * `String` — два C-аргумента (ptr, len): питоновский `str` маппится сюда —
///    как `&str` в Rust (`From<&str> for Value` → `Value::String` → `[pointer, usize]`).
///    `NUL` внутри — легален (длина явная),
///    как у Rust `&str`. NUL-терминированные API (`strlen`, `strcmp`, `printf`-fmt)
///    берите явно: `chillffi.CString("...")` — как литерал `c"..."` в Rust;
///  * `CString` — один C-аргумент (указатель на NUL-терминированный буфер):
///    только явный тип — `chillffi.CString(...)` (`pytypes.rs`) или ctypes `c_char_p`;
///  * `RawString` (`Vec<u8>`) — один C-аргумент (указатель на байты, без NUL):
///    питоновский `bytes` — как `b"..."` в Rust (`From<&[u8]>` → `Value::RawString`).
///
/// Полный набор ширин (U8..U64, I8..I64, Isize/Usize, F32) приходит из наших
/// pyclass-типов (`chillffi.I32(-7)` и т.д., `pytypes.rs` — зеркало `Type` из
/// `chillffi`) и из ctypes-инстансов (`ctypes.c_int32(5)` → `I32`, по `_type_`).
/// Голый int/float — конвертация по значению (`i64`/`u64`/`f64`).
#[derive(Debug, Clone)]
pub enum FfiArgValue
{
  U64(u64),
  U32(u32),
  U16(u16),
  U8(u8),
  Usize(usize),
  I64(i64),
  I32(i32),
  I16(i16),
  I8(i8),
  Isize(isize),
  F64(f64),
  F32(f32),
  Bool(bool),
  CString(std::ffi::CString),
  RawString(Vec<u8>),

  /// `Value::String` — utf8-байты; клон передаст C как (`ptr`, `len`).
  String(String),

  /// Зарегистрированный в клоне колбэк (`Value::Function(id)`).
  Callback(FfiCallback),

  /// Сырой адрес в пространстве клона (`Value::Pointer`) — `scope.alloc()`.
  Pointer(usize),

  /// Структура по значению (`Value::Struct`) — `chillffi.StructValue([...])`.
  /// Поля — те же `FfiArgValue`; `worker` сериализует в транзитный буфер
  /// (`structLayout` + `writeStructAt` на клоне). Полем строки могут быть только
  /// CString/RawString (один указатель); голый `str` (ptr+len) полем быть
  /// не может — как в Rust.
  Struct(Vec<FfiArgValue>)
}

/// Динамический аргумент → типизированный.
/// bool проверяется до int: в Python bool — подкласс int.
/// Наши типы (`chillffi.I32(...)` и т.д.) и ctypes-инстансы проверяются
/// до голых int/float — явный тип перебивает конвертацию по значению.
pub fn pyToFfiArg(object: Bound<'_, PyAny>) -> PyResult<FfiArgValue>
{
  if object.is_instance_of::<pyo3::types::PyBool>() {
    return Ok(FfiArgValue::Bool(object.extract::<bool>()?));
  }

  // Структура по значению (`chillffi.StructValue([...])`) → `Value::Struct`.
  // Вложенные структуры — рекурсивно (поля уже сконвертированы в #[new]).
  if let Ok(structRef) = object.cast::<crate::structs::PyStructValue>() {
    return Ok(FfiArgValue::Struct(structRef.borrow().fields.clone()));
  }

  // Зарегистрированный колбэк → `Value::Function(id)` (clone резолвит в code pointer).
  if let Ok(callbackRef) = object.cast::<PyCallback>() {
    return Ok(FfiArgValue::Callback(callbackRef.borrow().handle));
  }
  
  // Блок памяти клона (`scope.alloc`) → `Value::Pointer(address)`.
  if let Ok(memoryRef) = object.cast::<PyMemory>() {
    return Ok(FfiArgValue::Pointer(memoryRef.borrow().address));
  }

  // Наши типы-зеркала `Type` (`chillffi.I32(-7)`, `chillffi.String("...")`, ...).
  if let Some(arg) = typedArg(&object)? {
    return Ok(arg);
  }

  // ctypes-инстансы: тип берётся из `_type_` (`c_int32` → `i32`, `c_char_p` → `CString`,
  // `NULL`-указатели → `Pointer(0)`; хост-адреса отклоняются — изоляция).
  if let Some(arg) = ctypesArg(&object)? {
    return Ok(arg);
  }

  if object.is_instance_of::<pyo3::types::PyInt>() {
    return match object.extract::<i64>()
    {
      Ok(v) => Ok(FfiArgValue::I64(v)),
      Err(_) => Ok(FfiArgValue::U64(object.extract::<u64>()?))
    };
  }
  if object.is_instance_of::<pyo3::types::PyFloat>() {
    return Ok(FfiArgValue::F64(object.extract::<f64>()?));
  }
  
  // `bytes` → `RawString`: один указатель на байты, без `NUL` (бинарные буферы).
  if let Ok(b) = object.extract::<Vec<u8>>() {
    return Ok(FfiArgValue::RawString(b));
  }
  
  // `str` → `String` (`ptr`+`len`): один питоновский аргумент, клон развернёт в два
  // C-аргумента (`const char*`, `size_t`) — как `&str` в Rust (`Value::String`).
  // `NUL` внутри легален: длина идёт рядом, `NUL`-терминация не требуется.
  // `NUL`-терминированные параметры — явно `chillffi.CString(...)` (ветка выше).
  if let Ok(s) = object.extract::<String>()
  {
    return Ok(FfiArgValue::String(s));
  }
  Err(PyRuntimeError::new_err(format!(
    "unsupported argument type: {} (float/int/bool/str expected)",
    object.get_type().name()?
  )))
}

/// Один `FfiArgValue` → typed `builder.arg::<T>`. Общий макрос
/// для обычного и вариативного билдера — у обоих одинаковый набор `arg::<T>`.
macro_rules! pushTyped
{
  ($builder:expr, $arg:expr) =>
  {
    match $arg
    {
      FfiArgValue::U64(v)       => $builder.arg::<u64>(*v),
      FfiArgValue::U32(v)       => $builder.arg::<u32>(*v),
      FfiArgValue::U16(v)       => $builder.arg::<u16>(*v),
      FfiArgValue::U8(v)        => $builder.arg::<u8>(*v),
      FfiArgValue::Usize(v)     => $builder.arg::<usize>(*v),
      FfiArgValue::I64(v)       => $builder.arg::<i64>(*v),
      FfiArgValue::I32(v)       => $builder.arg::<i32>(*v),
      FfiArgValue::I16(v)       => $builder.arg::<i16>(*v),
      FfiArgValue::I8(v)        => $builder.arg::<i8>(*v),
      FfiArgValue::Isize(v)     => $builder.arg::<isize>(*v),
      FfiArgValue::F64(v)       => $builder.arg::<f64>(*v),
      FfiArgValue::F32(v)       => $builder.arg::<f32>(*v),
      FfiArgValue::Bool(v)      => $builder.arg::<bool>(*v),
      FfiArgValue::CString(v)   => $builder.arg::<std::ffi::CString>(v.clone()),
      FfiArgValue::RawString(v) => $builder.arg::<Vec<u8>>(v.clone()),
      FfiArgValue::String(v)    => $builder.arg::<String>(v.clone()),
      FfiArgValue::Callback(v)  => $builder.arg::<FfiCallback>(*v),
      FfiArgValue::Pointer(v)   => $builder.arg::<FfiPointer>(FfiPointer(*v)),
      // Структура по значению: поля → `Arg` ядра, собираем `StructValue` —
      // раскладку транзитного буфера сделает `worker` (`structLayout`).
      FfiArgValue::Struct(v)    => $builder.arg::<FfiStructValue>(
        FfiStructValue::new(v.iter().map(valueToArg)))
    }
  };
}

/// Приклеивает один аргумент к `CallBuilder` через typed `arg::<T>`.
pub fn pushArg<'a, 'g>(builder: CallBuilder<'a, 'g>, arg: &FfiArgValue) -> CallBuilder<'a, 'g>
{
  pushTyped!(builder, arg)
}

/// Тот же `pushArg` для вариативной части вызова.
pub fn pushVariadicArg<'a, 'g>(builder: VariadicCallBuilder<'a, 'g>, arg: &FfiArgValue) -> VariadicCallBuilder<'a, 'g>
{
  pushTyped!(builder, arg)
}

/// `FfiArgValue` → публичный `chillffi::Arg`. Общий конвертер для всех путей:
/// поля `StructValue`, значения `Scope::writeDynamicStruct` и пути через
/// `Scope::callPointer` (bridge.rs). `Struct` — рекурсивно.
pub(crate) fn valueToArg(value: &FfiArgValue) -> Arg
{
  match value
  {
    FfiArgValue::U64(v)      => Arg::from(*v),
    FfiArgValue::U32(v)      => Arg::from(*v),
    FfiArgValue::U16(v)      => Arg::from(*v),
    FfiArgValue::U8(v)       => Arg::from(*v),
    FfiArgValue::Usize(v)    => Arg::from(*v),
    FfiArgValue::I64(v)      => Arg::from(*v),
    FfiArgValue::I32(v)      => Arg::from(*v),
    FfiArgValue::I16(v)      => Arg::from(*v),
    FfiArgValue::I8(v)       => Arg::from(*v),
    FfiArgValue::Isize(v)    => Arg::from(*v),
    FfiArgValue::F64(v)      => Arg::from(*v),
    FfiArgValue::F32(v)      => Arg::from(*v),
    FfiArgValue::Bool(v)     => Arg::from(*v),
    FfiArgValue::CString(v)    => Arg::from(v.clone()),
    FfiArgValue::RawString(v)  => Arg::from(v.clone()),
    FfiArgValue::String(v)     => Arg::from(v.clone()),
    FfiArgValue::Callback(v)   => Arg::from(*v),
    FfiArgValue::Pointer(v)    => Arg::from(FfiPointer(*v)),
    FfiArgValue::Struct(v)     => Arg::from(FfiStructValue::new(v.iter().map(valueToArg)))
  }
}

// =================================================================================================
