use crate::args::{FfiArgValue, pushArg, pushVariadicArg, valueToArg};
use crate::scope::ScopeSlot;
use chillffi::ffi::errors::FFIError;
use chillffi::ffi::library::{CallBuilder, Library, VariadicCallBuilder};
use chillffi::ffi::scope::{FFIScope, Scope as ChillScope};
use chillffi::ffi::types::Type as FfiType;
use chillffi::ffi::types::primitive::{Arg as FfiArg, Pointer as FfiPointer, StructValue as FfiStructValue};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyAny;
// =================================================================================================
// bridge.rs — вызов символа и диспетчер типа результата.
//
// Библиотека грузится в удерживаемый scope на каждый вызов, аргументы
// идут через typed `.arg::<T>()`; токен результата приходит из финализатора
// `.result(...)` и всегда явный. `Expect` — единый интерфейс над обычным
// и вариативным билдером.
// =================================================================================================

/// Результат вызова в Rust-представлении; в Python конвертируется после detach.
#[derive(Debug)]
pub enum RetVal
{
  Float(f64),
  Int(i64),
  UInt(u64),
  Bool(bool),
  None
}

// todo desc
pub fn retValToPython(py: Python<'_>, outcome: Result<RetVal, FFIError>) -> PyResult<Py<PyAny>>
{
  match outcome
  {
    Ok(RetVal::Float(v)) => Ok(v.into_pyobject(py)?.into_any().unbind()),
    Ok(RetVal::Int(v)) => Ok(v.into_pyobject(py)?.into_any().unbind()),
    Ok(RetVal::UInt(v)) => Ok(v.into_pyobject(py)?.into_any().unbind()),
    // bool в pyo3 0.30 конвертируется в Borrowed<PyBool> (синглтон Py_True/Py_False)
    // — нужен to_owned() перед unbind().
    Ok(RetVal::Bool(v)) => Ok(pyo3::types::PyBool::new(py, v).to_owned().into_any().unbind()),
    Ok(RetVal::None) => Ok(py.None()),
    Err(error) => Err(PyRuntimeError::new_err(format!("chillffi: {error:?}")))
  }
}

// =================================================================================================
// bridge.rs: callPointerWithScope

/// Наши `FfiArgValue` → публичный `chillffi::Arg` для `Scope::callPointer`
/// (делегирует `valueToArg`, `args.rs`).
fn argToFfi(value: &FfiArgValue) -> FfiArg
{
  valueToArg(value)
}

/// Вызов по адресу в удерживаемом scope (`Scope::callPointer` /
/// `Scope::callPointerErrno`): адрес приходит извне — вернулся из
/// предыдущего вызова (`signal()`, `calloc`) или прочитан из памяти,
/// dlopen не участвует. Токен результата — тот же, что у `dispatch`.
pub fn callPointerWithScope(
  slot: &ScopeSlot,
  address: usize,
  args: Vec<FfiArgValue>,
  expect: &str,
  errno: bool
) -> Result<RetVal, FFIError>
{
  if !slot.isOpen() { return Err(FFIError::Other("scope is closed".to_string())); }

  let ffiArgs: Vec<FfiArg> = args.iter().map(argToFfi).collect();
  let expect: String = expect.to_string();

  slot.withScope(move |ffiScope: &FFIScope|
  {
    let scope: ChillScope<'_> = ffiScope.scope();

    macro_rules! typedCall
    {
      ($t:ty) =>
      {
        if errno { scope.callPointerErrno::<$t>(address, ffiArgs.clone()) }
        else     { scope.callPointer::<$t>(address, ffiArgs.clone()) }
      };
    }

    match expect.as_str()
    {
      "void" => typedCall!(()).map(|_| RetVal::None),
      "f64" | "double" => typedCall!(f64).map(RetVal::Float),
      "f32" | "float" => typedCall!(f32).map(|v| RetVal::Float(f64::from(v))),
      "i64" | "long" => typedCall!(i64).map(RetVal::Int),
      "i32" | "int" => typedCall!(i32).map(|v| RetVal::Int(i64::from(v))),
      "i16" => typedCall!(i16).map(|v| RetVal::Int(i64::from(v))),
      "i8" => typedCall!(i8).map(|v| RetVal::Int(i64::from(v))),
      "isize" => typedCall!(isize).map(|v| RetVal::Int(v as i64)),
      "u64" => typedCall!(u64).map(RetVal::UInt),
      "u32" => typedCall!(u32).map(|v| RetVal::UInt(u64::from(v))),
      "u16" => typedCall!(u16).map(|v| RetVal::UInt(u64::from(v))),
      "u8" => typedCall!(u8).map(|v| RetVal::UInt(u64::from(v))),
      "usize" => typedCall!(usize).map(|v| RetVal::UInt(v as u64)),
      // Адрес, который вернул C (`signal()`/`calloc`/...) — читается как
      // `Pointer`, наружу Python `int`.
      "pointer" | "ptr" => typedCall!(FfiPointer).map(|v| RetVal::UInt(v.0 as u64)),
      "bool" => typedCall!(bool).map(RetVal::Bool),
      other => Err(FFIError::Other(format!(
        "unknown ret type '{other}': expected f64|f32|i64|i32|i16|i8|isize|u64|u32|u16|u8|usize|pointer|bool|void"
      )))
    }
  })?
}

// =================================================================================================
// bridge.rs: callExternalWithScope

/// Загружает библиотеку в удерживаемом scope
/// и зовёт её метод. Один и тот же scope на всём протяжении `with`-блока
/// (Scope Retention) — вызовы внутри блока живут в одном клоне.
pub fn callExternalWithScope(
  slot: &ScopeSlot,
  libraryPath: &str,
  methodName: &str,
  args: Vec<FfiArgValue>,
  fixedArgs: Option<usize>,
  expect: &str,
  errno: bool
) -> Result<RetVal, FFIError>
{
  let expect: String = expect.to_string();
  withBuilder(slot, libraryPath, methodName, args, fixedArgs, errno,
    move |builder| dispatch(builder, &expect))
}

/// `.resultStruct(fields)`: тот же вызов, но финализатор возвращает структуру
/// по значению (`CallBuilder::resultStruct` / `VariadicCallBuilder::resultStruct`).
pub fn callExternalStructWithScope(
  slot: &ScopeSlot,
  libraryPath: &str,
  methodName: &str,
  args: Vec<FfiArgValue>,
  fixedArgs: Option<usize>,
  errno: bool,
  fields: Vec<FfiType>
) -> Result<FfiStructValue, FFIError>
{
  withBuilder(slot, libraryPath, methodName, args, fixedArgs, errno,
    move |builder| builder.resultStruct(&fields))
}

/// Общая сборка цепочки: библиотека в удерживаемом scope, фиксированные
/// аргументы → consuming `.variadic()` при заданной границе → errno-флаг.
/// Билдер заимствует локальные scope/lib — наружу не выходит: финализатор
/// применяется здесь же, внутри заимствования (замыкание получает готовый
/// билдер). Всё исполняется на потоке-владельце клона (`ScopeSlot::withScope`),
/// поэтому вызов безопасен с любого потока хоста.
fn withBuilder<R>(
  slot: &ScopeSlot,
  libraryPath: &str,
  methodName: &str,
  args: Vec<FfiArgValue>,
  fixedArgs: Option<usize>,
  errno: bool,
  finalize: impl FnOnce(Expect<'_, '_>) -> Result<R, FFIError> + Send + 'static
) -> Result<R, FFIError>
where
  R: Send + 'static
{
  if !slot.isOpen() { return Err(FFIError::Other("scope is closed".to_string())); }

  let split: usize = fixedArgs.unwrap_or(args.len());
  if fixedArgs.is_some() && split >= args.len()
  {
    return Err(FFIError::Other(
      "variadic call needs at least one argument after the fixed ones".to_string()
    ));
  }

  let libraryPath: String = libraryPath.to_string();
  let methodName: String = methodName.to_string();

  slot.withScope(move |ffiScope: &FFIScope|
  {
    let scope: ChillScope<'_> = ffiScope.scope();
    // Библиотека грузится на каждый вызов: dlopen клон кэширует по пути,
    // на стороне хоста — только реестр.
    let lib: Library<'_> = scope.load(&libraryPath)?;
    let mut builder: CallBuilder<'_, '_> = lib.call(&methodName);

    for arg in &args[..split]
    {
      builder = pushArg(builder, arg);
    }
    let errnoBuilder: Expect<'_, '_> = match fixedArgs
    {
      Some(_) =>
      {
        let mut variadic: VariadicCallBuilder<'_, '_> = builder.variadic();
        for arg in &args[split..]
        {
          variadic = pushVariadicArg(variadic, arg);
        }
        Expect::Variadic(if errno { variadic.errno() } else { variadic })
      }
      None => Expect::Normal(if errno { builder.errno() } else { builder })
    };
    finalize(errnoBuilder)
  })?
}

// -------------------------------------------------------------------------------------------------
// Диспетчер типа результата

/// Единый интерфейс над обычным и вариативным билдером — в `chillffi` у обоих
/// одинаковые финализаторы `.result::<T>()` / `.void()`.
enum Expect<'a, 'g>
{
  Normal(CallBuilder<'a, 'g>),
  Variadic(VariadicCallBuilder<'a, 'g>)
}

impl<'a, 'g> Expect<'a, 'g>
{
  fn result<T: chillffi::ffi::types::primitive::FfiPrimitive>(self) -> Result<T, FFIError>
  {
    match self
    {
      Expect::Normal(b) => b.result::<T>(),
      Expect::Variadic(b) => b.result::<T>()
    }
  }

  fn void(self) -> Result<(), FFIError>
  {
    match self
    {
      Expect::Normal(b) => b.void(),
      Expect::Variadic(b) => b.void()
    }
  }

  /// Финализатор структуры по значению (`resultStruct(&[Type])`):
  /// раскладка возвращаемого буфера объявлена вызывающим.
  fn resultStruct(self, fields: &[FfiType]) -> Result<FfiStructValue, FFIError>
  {
    match self
    {
      Expect::Normal(b) => b.resultStruct(fields),
      Expect::Variadic(b) => b.resultStruct(fields)
    }
  }
}

/// Тип результата выбирается диспетчером по токену,
/// пришедшему из `.result(...)` (строка / класс нашего типа / класс ctypes —
/// сводятся в один токен в `pytypes.rs` `expectToken`).
fn dispatch(builder: Expect<'_, '_>, expect: &str) -> Result<RetVal, FFIError>
{
  match expect
  {
    "f64" | "double" => builder.result::<f64>().map(RetVal::Float),
    "f32" | "float" => builder.result::<f32>().map(|v| RetVal::Float(f64::from(v))),
    "i64" | "long" => builder.result::<i64>().map(RetVal::Int),
    "i32" | "int" => builder.result::<i32>().map(|v| RetVal::Int(i64::from(v))),
    "i16" => builder.result::<i16>().map(|v| RetVal::Int(i64::from(v))),
    "i8" => builder.result::<i8>().map(|v| RetVal::Int(i64::from(v))),
    "isize" => builder.result::<isize>().map(|v| RetVal::Int(v as i64)),
    "u64" => builder.result::<u64>().map(RetVal::UInt),
    "u32" => builder.result::<u32>().map(|v| RetVal::UInt(u64::from(v))),
    "u16" => builder.result::<u16>().map(|v| RetVal::UInt(u64::from(v))),
    "u8" => builder.result::<u8>().map(|v| RetVal::UInt(u64::from(v))),
    "usize" => builder.result::<usize>().map(|v| RetVal::UInt(v as u64)),
    // Адрес, который вернул C (`malloc`/`calloc`/...) — читается как
    // `Pointer`, наружу Python `int`.
    "pointer" | "ptr" => builder.result::<FfiPointer>().map(|v| RetVal::UInt(v.0 as u64)),
    "bool" => builder.result::<bool>().map(RetVal::Bool),
    "void" => builder.void().map(|_| RetVal::None),
    other => Err(FFIError::Other(format!(
      "unknown ret type '{other}': expected f64|f32|i64|i32|i16|i8|isize|u64|u32|u16|u8|usize|pointer|bool|void"
    )))
  }
}

// =================================================================================================
