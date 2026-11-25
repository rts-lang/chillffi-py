// pytypes.rs — питоновские типы FFI: зеркало `Type` из `chillffi`
// (`chillffi.I32(-7)`, `chillffi.Pointer(...)`, строковые типы) + интероп
// с ctypes (инстансы как аргументы, классы в `.result()`).
// =================================================================================================
use crate::args::FfiArgValue;
use crate::memory::PyMemory;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyType};
// =================================================================================================
// Числовые типы: I8..I64, Isize, U8..U64, Usize, F32, F64, Bool

/// Реализует типизированный целочисленный pyclass:
/// Python int с проверкой диапазона (выход за границу — ошибка).
macro_rules! typedInt
{
  ($class:ident, $pyName:literal, $rust:ty, $min:expr, $max:expr) =>
  {
    #[doc = concat!("Явно типизированный FFI-аргумент `", $pyName,
                    "` — зеркало `chillffi::ffi::types::Type::", $pyName,
                    "` (как `arg::<", stringify!($rust), ">()` в Rust).\n\n",
                    "Значение — Python int; выход за диапазон — ошибка, не обрезка.")]
    #[pyclass(name = $pyName)]
    pub(crate) struct $class
    {
      value: $rust
    }

    #[pymethods]
    impl $class
    {
      /// Python `int` в диапазоне типа.
      #[new]
      fn new(value: i128) -> PyResult<Self>
      {
        if value < $min || value > $max
        {
          return Err(PyRuntimeError::new_err(format!(
            "chillffi.{} out of range: {} does not fit [{}, {}]",
            $pyName, value, $min, $max
          )));
        }
        Ok(Self { value: value as $rust })
      }

      /// Значение как Python `int`.
      #[getter]
      fn value(&self) -> i128
      {
        self.value as i128
      }

      /// Интероп с кодом, ждущим обычный `int`.
      fn __int__(&self) -> i128
      {
        self.value as i128
      }

      // todo desc
      fn __repr__(&self) -> String
      {
        format!("chillffi.{}({})", $pyName, self.value as i128)
      }
    }
  };
}

typedInt!(TypedI8,    "I8",    i8,     i8::MIN as i128,    i8::MAX as i128);
typedInt!(TypedI16,   "I16",   i16,    i16::MIN as i128,   i16::MAX as i128);
typedInt!(TypedI32,   "I32",   i32,    i32::MIN as i128,   i32::MAX as i128);
typedInt!(TypedI64,   "I64",   i64,    i64::MIN as i128,   i64::MAX as i128);
typedInt!(TypedIsize, "Isize", isize,  isize::MIN as i128, isize::MAX as i128);
typedInt!(TypedU8,    "U8",    u8,     0,                  u8::MAX as i128);
typedInt!(TypedU16,   "U16",   u16,    0,                  u16::MAX as i128);
typedInt!(TypedU32,   "U32",   u32,    0,                  u32::MAX as i128);
typedInt!(TypedU64,   "U64",   u64,    0,                  u64::MAX as i128);
typedInt!(TypedUsize, "Usize", usize,  0,                  usize::MAX as i128);

/// `F32`/`F64`: Python `float` — это `f64`; `F32` хранит и передаёт `f32`.
macro_rules! typedFloat
{
  ($class:ident, $pyName:literal, $rust:ty) =>
  {
    #[doc = concat!("Явно типизированный FFI-аргумент `", $pyName,
                    "` — как `arg::<", stringify!($rust), ">()` в Rust.\n\n",
                    "Принимает int и float; значение, не влезающее в ", $pyName,
                    ", становится inf (как у ctypes).")]
    #[pyclass(name = $pyName)]
    pub(crate) struct $class
    {
      value: $rust
    }

    #[pymethods]
    impl $class
    {
      /// Python int|float.
      #[new]
      fn new(value: f64) -> Self
      {
        Self { value: value as $rust }
      }

      /// Значение как Python float (f64).
      #[getter]
      fn value(&self) -> f64
      {
        self.value as f64
      }

      // todo desc
      fn __float__(&self) -> f64
      {
        self.value as f64
      }

      // todo desc
      fn __repr__(&self) -> String
      {
        format!("chillffi.{}({})", $pyName, self.value as f64)
      }
    }
  };
}

typedFloat!(TypedF32, "F32", f32);
typedFloat!(TypedF64, "F64", f64);

/// `Bool` — явный `arg::<bool>()`.
#[pyclass(name = "Bool")]
pub(crate) struct TypedBool
{
  // todo desc
  value: bool
}

#[pymethods]
impl TypedBool
{
  /// Python bool.
  #[new]
  fn new(value: bool) -> Self
  {
    Self { value }
  }

  // todo desc
  #[getter]
  fn value(&self) -> bool
  {
    self.value
  }

  // todo desc
  fn __bool__(&self) -> bool
  {
    self.value
  }

  // todo desc
  fn __repr__(&self) -> String
  {
    format!("chillffi.Bool({})", self.value)
  }
}

// =================================================================================================
// Pointer: сырой адрес в пространстве клона (Value::Pointer)

/// `Pointer` — один C-аргумент, сырой адрес (`void*`).
///
/// Адрес обязан быть из пространства клона: `scope.alloc()` (Memory) или
/// адрес, возвращённый C-функцией в этом же `with`-блоке (`malloc`/`calloc`/...,
/// читается `.result(Pointer)` как `int`). Адрес хоста в клоне невалиден — изоляция.
///
/// ```python
/// mem = scope.alloc(16)            # память клона
/// libc.memset.arg(Pointer(mem)).arg(65).arg(Usize(16)).result()
/// addr = libc.calloc.arg(4).arg(8).result(Pointer)  # адрес, вернувшийся из C
/// libc.free.arg(Pointer(addr)).result()             # освободить там же
/// ```
#[pyclass(name = "Pointer")]
pub(crate) struct TypedPointer
{
  /// Адрес в пространстве клона; `pub(crate)` — читается в `scope.pointer(addr)`.
  pub(crate) address: usize
}

#[pymethods]
impl TypedPointer
{
  /// Python `int` (адрес клона) или `Memory` (`scope.alloc`).
  #[new]
  fn new(value: &Bound<'_, PyAny>) -> PyResult<Self>
  {
    // Память клона → её адрес (то же, что голый Memory в `arg()`).
    if let Ok(memory) = value.cast::<PyMemory>() {
      return Ok(Self { address: memory.borrow().address });
    }

    let address: i128 = value.extract().map_err(|_|
      PyRuntimeError::new_err(
        "chillffi.Pointer: ожидаю int (адрес клона) или Memory (scope.alloc)"))?;

    if address < 0 {
      return Err(PyRuntimeError::new_err(format!(
        "chillffi.Pointer out of range: {address} отрицательный адрес")));
    }
    Ok(Self { address: address as usize })
  }

  /// Адрес как Python int.
  #[getter]
  fn value(&self) -> usize
  {
    self.address
  }

  // todo desc
  fn __int__(&self) -> usize
  {
    self.address
  }

  // todo desc
  fn __repr__(&self) -> String
  {
    format!("chillffi.Pointer(0x{:x})", self.address)
  }
}

// =================================================================================================
// Строковые типы: CString / RawString / String (ptr+len)

/// `CString` — один C-аргумент, `NUL`-терминированный (`const char*`).
/// Явный тип — как литерал `c"..."` в Rust: `NUL`-терминированные параметры
/// (`strlen`, `strcmp`, printf-fmt). Голый `str` уходит в `String` (ptr+len);
/// сюда же идёт ctypes `c_char_p`. Принимает и `bytes` для не-UTF8 C-строк.
#[pyclass(name = "CString")]
pub(crate) struct TypedCString
{
  // todo desc
  text: std::ffi::CString
}

#[pymethods]
impl TypedCString
{
  /// `str` (utf8) или `bytes`; `NUL` внутри — ошибка.
  #[new]
  fn new(value: &Bound<'_, PyAny>) -> PyResult<Self>
  {
    let bytes: Vec<u8> = if let Ok(text) = value.extract::<String>() {
      text.into_bytes()
    } else {
      value.extract().map_err(|_|
        PyRuntimeError::new_err("chillffi.CString: ожидаю str или bytes"))?
    };

    let text: std::ffi::CString = std::ffi::CString::new(bytes).map_err(|_|
      PyRuntimeError::new_err("chillffi.CString: interior NUL byte"))?;

    Ok(Self { text })
  }

  /// Байты без `NUL` — как `.value` у `ctypes.c_char_p`.
  #[getter]
  fn value(&self) -> Vec<u8>
  {
    self.text.to_bytes().to_vec()
  }

  // todo desc
  fn __repr__(&self) -> String
  {
    format!("chillffi.CString({:?})", self.text.to_string_lossy())
  }
}

/// `RawString` — один C-аргумент, указатель на байты без `NUL` (`uint8_t*`).
/// Голый `bytes` в `arg()` маппится сюда же. Длину C-сторона знает сама —
/// если нужен `(ptr, len)`, это `chillffi.String`.
#[pyclass(name = "RawString")]
pub(crate) struct TypedRawString
{
  // todo desc
  bytes: Vec<u8>
}

#[pymethods]
impl TypedRawString
{
  /// `bytes`.
  #[new]
  fn new(bytes: Vec<u8>) -> Self
  {
    Self { bytes }
  }

  // todo desc
  #[getter]
  fn value(&self) -> Vec<u8>
  {
    self.bytes.clone()
  }

  // todo desc
  fn __repr__(&self) -> String
  {
    format!("chillffi.RawString({:?})", self.bytes)
  }
}

/// `String` — два C-аргумента `(ptr, len)` — `const char*, size_t`.
/// Один питоновский аргумент разворачивается в два C-аргумента силами
/// `chillffi` (`Value::String`). Голый `str` в `arg()` маппится сюда же
/// (как `&str` в Rust): `lib.call("strnlen").arg("hello world")` вместо
/// ручного `arg(mem).arg(len)`. Явный тип — когда вид типа нужен в вызове.
///
/// Только `str` (utf8): произвольные байты — это `RawString`/`Memory`.
#[pyclass(name = "String")]
pub(crate) struct TypedString
{
  // todo desc
  text: String
}

#[pymethods]
impl TypedString
{
  /// `str` (utf8).
  #[new]
  fn new(text: String) -> Self
  {
    Self { text }
  }

  // todo desc
  #[getter]
  fn value(&self) -> String
  {
    self.text.clone()
  }

  // todo desc
  fn __repr__(&self) -> String
  {
    format!("chillffi.String({:?})", self.text)
  }
}

// =================================================================================================
// Наши типы → FfiArgValue

/// Наши pyclass-значения → `FfiArgValue`. Явный тип перебивает конвертацию
/// по значению: `chillffi.I32(-7)` уходит как `arg::<i32>()`, а не как `i64`.
pub(crate) fn typedArg(object: &Bound<'_, PyAny>) -> PyResult<Option<FfiArgValue>>
{
  return
  if let Ok(v) = object.cast::<TypedI8>()             { Ok(Some(FfiArgValue::I8(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedI16>()       { Ok(Some(FfiArgValue::I16(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedI32>()       { Ok(Some(FfiArgValue::I32(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedI64>()       { Ok(Some(FfiArgValue::I64(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedIsize>()     { Ok(Some(FfiArgValue::Isize(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedU8>()        { Ok(Some(FfiArgValue::U8(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedU16>()       { Ok(Some(FfiArgValue::U16(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedU32>()       { Ok(Some(FfiArgValue::U32(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedU64>()       { Ok(Some(FfiArgValue::U64(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedUsize>()     { Ok(Some(FfiArgValue::Usize(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedF32>()       { Ok(Some(FfiArgValue::F32(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedF64>()       { Ok(Some(FfiArgValue::F64(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedBool>()      { Ok(Some(FfiArgValue::Bool(v.borrow().value))) }
  else if let Ok(v) = object.cast::<TypedPointer>()   { Ok(Some(FfiArgValue::Pointer(v.borrow().address))) }
  else if let Ok(v) = object.cast::<TypedCString>()   { Ok(Some(FfiArgValue::CString(v.borrow().text.clone()))) }
  else if let Ok(v) = object.cast::<TypedRawString>() { Ok(Some(FfiArgValue::RawString(v.borrow().bytes.clone()))) }
  else if let Ok(v) = object.cast::<TypedString>()    { Ok(Some(FfiArgValue::String(v.borrow().text.clone()))) }
  else                                                { Ok(None) }
}

// =================================================================================================
// ctypes-инстансы → FfiArgValue (интероп)

/// `ctypes`-инстанс → `FfiArgValue`; `None` — «это не ctypes», идём в обычные ветки.
///
/// Таблица по `_type_` (коду `ctypes._SimpleCData`):
///   '?'→Bool, 'b/B/h/H/i/I/l/L/q/Q'→целые нужной ширины ('l/L' — c_long:
///   64 бита на unix LP64, 32 на Windows), 'f'→F32, 'd'→F64,
///   'z'→CString (None → NULL-указатель), 'P'→Pointer (только NULL).
///
/// Не поддержаны и отклоняются с объяснением: 'c'/'u'/'Z' (в `chillffi` нет
/// wide-string и одиночного c_char), ненулевые 'P' и `_Pointer` (адрес хоста
/// в клоне невалиден — изоляция), `byref()` (туда же), `Array`/`Structure`
/// (упаковка — через `Memory`; структуры по значению — `chillffi.StructValue([...])`).
pub(crate) fn ctypesArg(object: &Bound<'_, PyAny>) -> PyResult<Option<FfiArgValue>>
{
  // `_type_` есть у инстансов `ctypes._SimpleCData` (унаследован от класса).
  let code: String = match object.getattr("_type_")
  {
    Ok(code) => match code.extract::<String>()
    {
      // `POINTER(c_int)._type_` — класс pointed-to, не символ — не наш случай.
      Ok(code) if code.len() == 1 => code,
      _ => return nonCdataArg(object)
    },
    Err(_) => return nonCdataArg(object)
  };

  // `.value` есть у всех `_SimpleCData`: `int`/`float`/`bool`/`bytes`/`None`.
  let value: Bound<'_, PyAny> = object.getattr("value")?;

  match code.as_str()
  {
    "?" => Ok(Some(FfiArgValue::Bool(value.extract::<bool>()?))),

    "b" => Ok(Some(FfiArgValue::I8(value.extract::<i64>()? as i8))),
    "h" => Ok(Some(FfiArgValue::I16(value.extract::<i64>()? as i16))),
    "i" => Ok(Some(FfiArgValue::I32(value.extract::<i64>()? as i32))),
    "q" => Ok(Some(FfiArgValue::I64(value.extract::<i64>()?))),
    "l" =>
    {
      // `c_long`: 64 бита на unix (LP64), 32 на Windows — как там его объявляет `ctypes`.
      if cfg!(target_os = "windows") {
        Ok(Some(FfiArgValue::I32(value.extract::<i64>()? as i32)))
      } else {
        Ok(Some(FfiArgValue::I64(value.extract::<i64>()?)))
      }
    }

    "B" => Ok(Some(FfiArgValue::U8(value.extract::<i64>()? as u8))),
    "H" => Ok(Some(FfiArgValue::U16(value.extract::<i64>()? as u16))),
    "I" => Ok(Some(FfiArgValue::U32(value.extract::<i64>()? as u32))),
    "Q" => Ok(Some(FfiArgValue::U64(value.extract::<u64>()?))),
    "L" =>
    {
      if cfg!(target_os = "windows") {
        Ok(Some(FfiArgValue::U32(value.extract::<i64>()? as u32)))
      } else {
        Ok(Some(FfiArgValue::U64(value.extract::<u64>()?)))
      }
    }

    "f" => Ok(Some(FfiArgValue::F32(value.extract::<f64>()? as f32))),
    "d" => Ok(Some(FfiArgValue::F64(value.extract::<f64>()?))),

    // `c_char_p`: `None` → `NULL`, `bytes` → `CString` (один указатель, `NUL` снаружи уже стоит).
    "z" =>
    {
      if value.is_none() {
        Ok(Some(FfiArgValue::Pointer(0)))
      } else
      {
        let bytes: Vec<u8> = value.extract()?;
        let text: std::ffi::CString = std::ffi::CString::new(bytes).map_err(|_|
          PyRuntimeError::new_err("ctypes c_char_p: interior NUL byte"))?;
        Ok(Some(FfiArgValue::CString(text)))
      }
    }

    // `c_void_p`: только `NULL` — ненулевое значение это адрес хоста.
    "P" =>
    {
      if value.is_none() {
        return Ok(Some(FfiArgValue::Pointer(0)));
      }
      let address: u64 = value.extract()?;
      if address == 0 {
        Ok(Some(FfiArgValue::Pointer(0)))
      } else
      {
        Err(PyRuntimeError::new_err(
          "ctypes c_void_p с ненулевым адресом: адрес хоста невалиден в клоне \
           (изоляция процессов) — используй scope.alloc() (Memory) или адрес, \
           полученный от C в этом же with-блоке: chillffi.Pointer(addr)"
        ))
      }
    }

    "c" | "u" | "Z" => Err(PyRuntimeError::new_err(format!(
      "ctypes _type_ '{code}' не поддержан: в chillffi нет одиночного c_char и wide-string \
       — используй chillffi.I8/U8, chillffi.CString(str) или chillffi.RawString(bytes)"
    ))),

    other => Err(PyRuntimeError::new_err(format!(
      "ctypes _type_ '{other}' не поддержан (нужны числовые, bool, c_char_p, c_void_p)"
    )))
  }
}

/// Не `_SimpleCData`: `byref()`, `Array`, `Structure`, `POINTER(...)` — свой разбор.
fn nonCdataArg(object: &Bound<'_, PyAny>) -> PyResult<Option<FfiArgValue>>
{
  // byref(obj) — CArgObject: указатель на память хоста, в клоне невалиден.
  let typeName: String = object.get_type().name()?.to_string();
  if typeName == "CArgObject"
  {
    return Err(PyRuntimeError::new_err(
      "ctypes.byref(...) даёт адрес хоста — в клоне невалиден (изоляция процессов). \
       Память клона: scope.alloc(n) → .arg(mem) / .arg(chillffi.Pointer(mem))"
    ));
  }

  // Дальше нужен `ctypes`: `Array` / `Structure` / `_Pointer`.
  let Ok(ctypes) = object.py().import("ctypes") else { return Ok(None) };

  let isArray: bool = ctypes
    .getattr("Array")
    .and_then(|t| object.is_instance(&t))
    .unwrap_or(false);
  if isArray
  {
    return Err(PyRuntimeError::new_err(
      "ctypes.Array: данные в памяти хоста — в клоне невалидны. \
       Скопируй в память клона: mem = scope.alloc(n); mem.write(bytes)"
    ));
  }

  let isStructure: bool = ctypes
    .getattr("Structure")
    .and_then(|t| object.is_instance(&t))
    .unwrap_or(false);
  if isStructure
  {
    return Err(PyRuntimeError::new_err(
      "ctypes.Structure: инстанс уходит в память хоста, в клоне невалиден. \
       Структуры по значению — chillffi.StructValue([...]); раскладка буфера — \
       Memory.readStruct/writeStruct (examples/byValueStruct, reprStruct)"
    ));
  }

  let isPointer: bool = ctypes
    .getattr("_Pointer")
    .and_then(|t| object.is_instance(&t))
    .unwrap_or(false);
  if isPointer
  {
    // `ctypes.cast(p, c_void_p).value` — адрес, на который указывает `POINTER(...)`.
    let cast: Bound<'_, PyAny> =
      ctypes.call_method1("cast", (object, ctypes.getattr("c_void_p")?))?;
    let target: Bound<'_, PyAny> = cast.getattr("value")?;

    return if target.is_none()
    {
      Ok(Some(FfiArgValue::Pointer(0))) // `NULL`-указатель — легален
    }
    else
    {
      Err(PyRuntimeError::new_err(
        "ctypes.POINTER(...) с ненулевым адресом: адрес хоста невалиден в клоне \
         (изоляция процессов) — используй scope.alloc() / адрес, полученный от C"
      ))
    };
  }

  Ok(None)
}

// =================================================================================================
// .result(): строка | класс нашего типа | класс ctypes → токен диспетчера

/// Класс нашего типа → токен `.result()`.
fn expectTokenOfName(name: &str) -> Option<&'static str>
{
  match name
  {
    "I8" => Some("i8"),
    "I16" => Some("i16"),
    "I32" => Some("i32"),
    "I64" => Some("i64"),
    "Isize" => Some("isize"),
    "U8" => Some("u8"),
    "U16" => Some("u16"),
    "U32" => Some("u32"),
    "U64" => Some("u64"),
    "Usize" => Some("usize"),
    "F32" => Some("f32"),
    "F64" => Some("f64"),
    "Bool" => Some("bool"),
    "Pointer" => Some("pointer"),
    _ => None
  }
}

/// Код `ctypes._type_` класса → токен `.result()`. Результат-тип — то, что читается
/// из регистра: числовые, `bool` и адреса (`'z'/'P'/'Z'` → `pointer`).
fn ctypesExpectToken(code: &str) -> Option<&'static str>
{
  match code
  {
    "?" => Some("bool"),
    "b" => Some("i8"),
    "h" => Some("i16"),
    "i" => Some("i32"),
    "q" => Some("i64"),
    "B" => Some("u8"),
    "H" => Some("u16"),
    "I" => Some("u32"),
    "Q" => Some("u64"),
    "f" => Some("f32"),
    "d" => Some("f64"),
    "z" | "P" | "Z" => Some("pointer"),
    "l" =>
    {
      if cfg!(target_os = "windows") {
        Some("i32")
      } else {
        Some("i64")
      }
    }
    "L" =>
    {
      if cfg!(target_os = "windows") {
        Some("u32")
      } else {
        Some("u64")
      }
    }
    _ => None
  }
}

/// Известные диспетчеру токены результата. Проверка здесь,
/// чтобы опечатка ловилась на `.result("i3")`, а не на вызове.
const ExpectTokens: [&str; 20] =
[
  "f64", "double", "f32", "float",
  "i64", "long", "i32", "int", "i16", "i8", "isize",
  "u64", "u32", "u16", "u8", "usize",
  "pointer", "ptr", "bool", "void"
];

/// `.result(x)`: x — строковый токен (`"i32"`), класс нашего типа (`chillffi.I32`)
/// или класс ctypes (`ctypes.c_int32`). Возвращает токен диспетчера `dispatch`.
///
/// Строковые типы результатом не читаются: адрес — `.result(Pointer)`,
/// байты — из `Memory`. Инстансы наших типов в `.result()` не проходят —
/// там нужен тип, не значение.
pub(crate) fn expectToken(object: &Bound<'_, PyAny>) -> PyResult<String>
{
  // 1. Строковый токен — как раньше: `.result("i32")`.
  if let Ok(token) = object.extract::<String>()
  {
    return if ExpectTokens.contains(&token.as_str()) {
      Ok(token)
    } else {
      Err(PyRuntimeError::new_err(format!(
        ".result(): unknown result type '{token}': expected f64|f32|i64|i32|i16|i8|isize|u64|u32|u16|u8|usize|pointer|bool|void"
      )))
    };
  }

  // 2. Класс `ctypes`: `.result(ctypes.c_int32)` — тип из `_type_`.
  if let Ok(code) = object.getattr("_type_")
  {
    if let Ok(code) = code.extract::<String>()
    {
      if code.len() == 1
      {
        return ctypesExpectToken(&code)
          .map(|token| token.to_string())
          .ok_or_else(|| {
            PyRuntimeError::new_err(format!(
              ".result(): ctypes-тип с _type_ '{code}' не читается как результат \
               (нужны числовые, bool; char*/void* — это адрес, .result(Pointer))"
            ))
          });
      }
    }
  }

  // 3. Класс нашего типа: `.result(chillffi.I32)`.
  if object.is_instance_of::<PyType>()
  {
    let name: String = object.getattr("__name__")?.extract()?;

    if let Some(token) = expectTokenOfName(&name) {
      return Ok(token.to_string());
    }

    return if matches!(name.as_str(), "CString" | "RawString" | "String")
    {
      Err(PyRuntimeError::new_err(format!(
        ".result(): строковые типы ({name}) не читаются как результат — \
         адрес верни .result(Pointer), байты читай из Memory"
      )))
    } else
    {
      Err(PyRuntimeError::new_err(format!(
        ".result(): chillffi-тип '{name}' не бывает результатом"
      )))
    };
  }

  Err(PyRuntimeError::new_err(format!(
    ".result(): жду строковый токен ('i32'), класс chillffi (I32) или класс ctypes \
     (c_int32), получил {}",
    object.get_type().name()?
  )))
}

// =================================================================================================
