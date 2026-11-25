use crate::scope::ScopeSlot;
use chillffi::ffi::callback::addressing::{relativeOffsetOf, tagOf, typesTagOf};
use chillffi::ffi::callback::sendable::Sendable;
use chillffi::ffi::callback::{CallError, ErasedCallable};
use chillffi::ffi::types::Type as FfiType;
use chillffi::ffi::types::primitive::{Callback as FfiCallback, DynamicList, Pointer as FfiPointer};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyDict, PyModule, PyTuple};
use std::sync::Arc;
// =================================================================================================
// callback.rs — питоновская функция как C function pointer.
//
// Тело уезжает в клон маршаленным кодом + pickled-окружением и исполняется там
// же, где живут данные. Хост-сторона — `buildCallbackSendable`, клон-сторона —
// `pyCallbackDecode` + `runCallbackPython`.
// =================================================================================================

/// Тег сайта колбэка: декодер `pyCallbackDecode` применяется в клоне
/// только к payload с этим тегом.
const CALLBACK_SITE_TAG: &str = "chillffi-py:pyCallback";
const CALLBACK_CODE_MAX: usize = 4096;
const CALLBACK_DEFAULTS_MAX: usize = 512;
const CALLBACK_GLOBALS_MAX: usize = 1024;

// Коды типов в payload (своя кодировка — байты едят бит-копией).
const TAG_VOID: u8 = 0;
const TAG_I8: u8 = 1;
const TAG_I16: u8 = 2;
const TAG_I32: u8 = 3;
const TAG_I64: u8 = 4;
const TAG_ISIZE: u8 = 5;
const TAG_U8: u8 = 6;
const TAG_U16: u8 = 7;
const TAG_U32: u8 = 8;
const TAG_U64: u8 = 9;
const TAG_USIZE: u8 = 10;
const TAG_F32: u8 = 11;
const TAG_F64: u8 = 12;
const TAG_BOOL: u8 = 13;
const TAG_POINTER: u8 = 14;

/// Состояние «closure» для `Sendable::fromClosure`: POD-структура (Copy +
/// Send), бит-копия которой пересекает fork. Содержит marshal-байткод
/// функции и её pickled-окружение.
#[derive(Clone, Copy)]
struct PyCallbackState
{
  argCount: u8,
  argTypes: [u8; 8],
  returnType: u8,
  codeLen: u32,
  defaultsLen: u32,
  globalsLen: u32,
  code: [u8; CALLBACK_CODE_MAX],
  defaults: [u8; CALLBACK_DEFAULTS_MAX],
  globals: [u8; CALLBACK_GLOBALS_MAX]
}

impl PyCallbackState
{
  // todo desc
  const fn empty() -> Self
  {
    Self
    {
      argCount: 0,
      argTypes: [0; 8],
      returnType: TAG_VOID,
      codeLen: 0,
      defaultsLen: 0,
      globalsLen: 0,
      code: [0; CALLBACK_CODE_MAX],
      defaults: [0; CALLBACK_DEFAULTS_MAX],
      globals: [0; CALLBACK_GLOBALS_MAX]
    }
  }

  /// Токен сигнатуры → (код payload, тег `chillffi` `Type`).
  fn typeToken(token: &str) -> Option<(u8, FfiType)>
  {
    match token
    {
      "void" | "none" => Some((TAG_VOID, FfiType::None)),
      "i8" => Some((TAG_I8, FfiType::I8)),
      "i16" => Some((TAG_I16, FfiType::I16)),
      "i32" => Some((TAG_I32, FfiType::I32)),
      "i64" => Some((TAG_I64, FfiType::I64)),
      "isize" => Some((TAG_ISIZE, FfiType::Isize)),
      "u8" => Some((TAG_U8, FfiType::U8)),
      "u16" => Some((TAG_U16, FfiType::U16)),
      "u32" => Some((TAG_U32, FfiType::U32)),
      "u64" => Some((TAG_U64, FfiType::U64)),
      "usize" => Some((TAG_USIZE, FfiType::Usize)),
      "f32" => Some((TAG_F32, FfiType::F32)),
      "f64" => Some((TAG_F64, FfiType::F64)),
      "bool" => Some((TAG_BOOL, FfiType::Bool)),
      "pointer" | "ptr" => Some((TAG_POINTER, FfiType::Pointer)),
      _ => None
    }
  }

  // todo desc
  fn stateTagToType(tag: u8) -> Result<FfiType, CallError>
  {
    let token: &str = match tag
    {
      TAG_VOID => "void",
      TAG_I8 => "i8",
      TAG_I16 => "i16",
      TAG_I32 => "i32",
      TAG_I64 => "i64",
      TAG_ISIZE => "isize",
      TAG_U8 => "u8",
      TAG_U16 => "u16",
      TAG_U32 => "u32",
      TAG_U64 => "u64",
      TAG_USIZE => "usize",
      TAG_F32 => "f32",
      TAG_F64 => "f64",
      TAG_BOOL => "bool",
      TAG_POINTER => "pointer",
      other => return Err(CallError::Decode(format!("chillffi-py callback: unknown type tag {other}")))
    };
    Ok(Self::typeToken(token).expect("token mapped above").1)
  }
}

/// Тип сигнатуры колбэка: [(код payload, тег `chillffi` `Type`)...] + возвращаемый.
pub type CallbackSignature = (Vec<(u8, FfiType)>, (u8, FfiType));

/// Парсер сигнатуры `"ret(arg1,arg2,...)"` — токены как у `.result()`.
pub fn parseCallbackSignature(signature: &str) -> PyResult<CallbackSignature>
{
  let (returnPart, argsPart): (&str, &str) = match signature.split_once('(')
  {
    Some((ret, args)) => (ret.trim(), args),
    None => (signature.trim(), "")
  };
  let argsPart: &str = argsPart.trim_end_matches(')').trim();

  let returnType: (u8, FfiType) = PyCallbackState::typeToken(returnPart).ok_or_else(||
    PyRuntimeError::new_err(format!(
      "unknown callback return type '{returnPart}': expected i8..i64|isize|u8..u64|usize|f32|f64|bool|pointer|void"
    ))
  )?;

  let mut argTypes: Vec<(u8, FfiType)> = Vec::new();
  for token in argsPart.split(',')
  {
    let token: &str = token.trim();
    if token.is_empty() {
      continue;
    }
    let parsed: (u8, FfiType) = PyCallbackState::typeToken(token).ok_or_else(||
      PyRuntimeError::new_err(format!(
        "unknown callback argument type '{token}': expected i8..i64|isize|u8..u64|usize|f32|f64|bool|pointer"
      ))
    )?;
    argTypes.push(parsed);
  }
  if argTypes.len() > 8
  {
    return Err(PyRuntimeError::new_err(
      "chillffi-py callback supports at most 8 arguments"
    ));
  }
  Ok((argTypes, returnType))
}

/// Хост-сторона: собирает `Sendable` из питоновской функции.
///
/// Бит-копией тело переправить нельзя (`PyObject` — куча хоста), поэтому
/// оно уезжает маршаленным кодом + pickled-окружением и исполняется в
/// клоне тем же интерпретатором, что поднял зиготу. Указатели, которые C
/// кладёт в аргументы, валидны в пространстве клона — тело разыменовывает
/// их ctypes-ом.
///
/// Ограничения (аналог «captures строго Copy» в `chillffi`):
///  * без free-переменных (`co_freevars` пуст) — захваты через default-args
///    или глобальные имена уровня модуля (снимок `co_names ∩ __globals__`
///    уезжает pickle-ом);
///  * тело не должно звать `chillffi` (оно выполняется внутри C-вызова,
///    worker клона занят);
///  * исключение Python → traceback в stderr клона → вызов возвращает
///    `FFIError` ("a registered callback panicked") — как паника closure
///    в `chillffi`.
pub fn buildCallbackSendable(
  py: Python<'_>,
  callbackFn: &Bound<'_, PyAny>,
  argTypes: Vec<(u8, FfiType)>,
  returnType: (u8, FfiType)
) -> PyResult<Sendable>
{
  // Тело должно быть честной функцией: __code__ / __defaults__ / __globals__.
  let codeObj: Bound<'_, PyAny> = callbackFn.getattr("__code__").map_err(|_|
    PyRuntimeError::new_err(
      "callback must be a plain function or lambda (with __code__): functools.partial and callable objects are not supported"
    )
  )?;
  let freevars: Vec<String> = codeObj.getattr("co_freevars")?.extract()?;
  if !freevars.is_empty()
  {
    return Err(PyRuntimeError::new_err(format!(
      "callback captures free variables {freevars:?}: not supported — use default arguments or module-level globals"
    )));
  }

  // Тело: marshal(code) — то, что реально исполнится в клоне.
  let marshalMod: Bound<'_, PyModule> = py.import("marshal")?;
  let codeBytes: Vec<u8> = marshalMod.call_method1("dumps", (codeObj.clone(),))?.extract()?;
  if codeBytes.len() > CALLBACK_CODE_MAX
  {
    return Err(PyRuntimeError::new_err(format!(
      "callback code is {} bytes, limit is {} (marshal)", codeBytes.len(), CALLBACK_CODE_MAX
    )));
  }

  // Захваты через default-args: pickle(__defaults__).
  let pickled: Vec<u8> =
  {
    let pickleMod: Bound<'_, PyModule> = py.import("pickle")?;
    let defaults: Bound<'_, PyAny> = callbackFn.getattr("__defaults__")?;
    if defaults.is_none() || defaults.len()? == 0 {
      Vec::new()
    } else
    {
      let bytes: Vec<u8> = pickleMod.call_method1("dumps", (defaults,))?.extract()?;
      if bytes.len() > CALLBACK_DEFAULTS_MAX
      {
        return Err(PyRuntimeError::new_err(format!(
          "callback defaults are {} bytes, limit is {} (pickle)", bytes.len(), CALLBACK_DEFAULTS_MAX
        )));
      }
      bytes
    }
  };

  // Захваты через глобальные имена: снимок co_names ∩ __globals__ (pickle).
  // Это питоновский аналог «captures строго Copy» из `chillffi`: значения
  // должны быть picklable (int/str/bool/...).
  let globalsBytes: Vec<u8> =
  {
    let pickleMod: Bound<'_, PyModule> = py.import("pickle")?;
    let names: Vec<String> = codeObj.getattr("co_names")?.extract()?;
    let fnGlobals: Bound<'_, PyDict> = callbackFn.getattr("__globals__")?.cast_into()?;
    let snapshot: Bound<'_, PyDict> = PyDict::new(py);
    for name in &names
    {
      // В снимок идут только picklable-значения: имена импортов (модули)
      // в co_names попадают, но не сериализуются — тело импортирует их само.
      if let Some(value) = fnGlobals.get_item(name)?
      {
        if pickleMod.call_method1("dumps", (value.clone(),)).is_ok() {
          snapshot.set_item(name, value)?;
        }
      }
    }
    if snapshot.len() == 0 {
      Vec::new()
    } else
    {
      let bytes: Vec<u8> = pickleMod.call_method1("dumps", (snapshot,))?.extract()?;
      if bytes.len() > CALLBACK_GLOBALS_MAX
      {
        return Err(PyRuntimeError::new_err(format!(
          "callback globals snapshot is {} bytes, limit is {} (pickle)", bytes.len(), CALLBACK_GLOBALS_MAX
        )));
      }
      bytes
    }
  };

  let mut state: PyCallbackState = PyCallbackState::empty();
  state.argCount = argTypes.len() as u8;
  for (i, (tag, _)) in argTypes.iter().enumerate() {
    state.argTypes[i] = *tag;
  }
  state.returnType = returnType.0;
  state.codeLen = codeBytes.len() as u32;
  state.defaultsLen = pickled.len() as u32;
  state.globalsLen = globalsBytes.len() as u32;
  state.code[..codeBytes.len()].copy_from_slice(&codeBytes);
  state.defaults[..pickled.len()].copy_from_slice(&pickled);
  state.globals[..globalsBytes.len()].copy_from_slice(&globalsBytes);

  // Тот же механизм, что у `callback!` внутри: fn-указатель декодера этого
  // `.so` → смещение относительно moduleBase; клон разрешает его по своей базе
  // того же `.so` и вызывает наш `decodeImpl`.
  type DecodeFnSignature = fn(u64, u64, &[u8]) -> Result<ErasedCallable, CallError>;
  let decodeFn: DecodeFnSignature = pyCallbackDecode;
  let relativeOffset: usize = relativeOffsetOf(decodeFn as *const () as usize);
  let siteTag: u64 = tagOf(CALLBACK_SITE_TAG);
  let ffiArgTypes: Vec<FfiType> = argTypes.into_iter().map(|(_, t)| t).collect();

  Ok(Sendable::fromClosure(
    state,
    relativeOffset,
    siteTag,
    ffiArgTypes,
    returnType.1
  ))
}

/// Клон-сторона: вызывается из `chillffi::ffi::callback::decode` (`worker.rs`,
/// при `RegisterCallback`) через разрешённый по moduleBase fn-указатель.
/// Проверяет оба тега — как `decodeImpl` из `callback!` — и собирает
/// `ErasedCallable` с конкретным Output-типом по returnType.
fn pyCallbackDecode(
  siteTag: u64,
  argsOutputTag: u64,
  bytes: &[u8]
) -> Result<ErasedCallable, CallError>
{
  if siteTag != tagOf(CALLBACK_SITE_TAG) {
    return Err(CallError::TypeMismatch { tag: siteTag });
  }
  if bytes.len() != size_of::<PyCallbackState>()
  {
    return Err(CallError::Decode(format!(
      "chillffi-py callback state size mismatch: expected {}, got {}",
      size_of::<PyCallbackState>(),
      bytes.len()
    )));
  }
  let state: PyCallbackState = unsafe{ std::ptr::read_unaligned(bytes.as_ptr().cast::<PyCallbackState>()) };

  let mut argTypes: Vec<FfiType> = Vec::with_capacity(state.argCount as usize);
  for tag in &state.argTypes[..state.argCount as usize] {
    argTypes.push(PyCallbackState::stateTagToType(*tag)?);
  }
  let returnType: FfiType = PyCallbackState::stateTagToType(state.returnType)?;
  if argsOutputTag != typesTagOf(&argTypes, &returnType) {
    return Err(CallError::ArgsOutputMismatch);
  }

  Ok(match returnType
  {
    FfiType::None => ErasedCallable::fromStateAndFn(state, callPyCallback::<()>),
    FfiType::I8 => ErasedCallable::fromStateAndFn(state, callPyCallback::<i8>),
    FfiType::I16 => ErasedCallable::fromStateAndFn(state, callPyCallback::<i16>),
    FfiType::I32 => ErasedCallable::fromStateAndFn(state, callPyCallback::<i32>),
    FfiType::I64 => ErasedCallable::fromStateAndFn(state, callPyCallback::<i64>),
    FfiType::Isize => ErasedCallable::fromStateAndFn(state, callPyCallback::<isize>),
    FfiType::U8 => ErasedCallable::fromStateAndFn(state, callPyCallback::<u8>),
    FfiType::U16 => ErasedCallable::fromStateAndFn(state, callPyCallback::<u16>),
    FfiType::U32 => ErasedCallable::fromStateAndFn(state, callPyCallback::<u32>),
    FfiType::U64 => ErasedCallable::fromStateAndFn(state, callPyCallback::<u64>),
    FfiType::Usize => ErasedCallable::fromStateAndFn(state, callPyCallback::<usize>),
    FfiType::F32 => ErasedCallable::fromStateAndFn(state, callPyCallback::<f32>),
    FfiType::F64 => ErasedCallable::fromStateAndFn(state, callPyCallback::<f64>),
    FfiType::Bool => ErasedCallable::fromStateAndFn(state, callPyCallback::<bool>),
    FfiType::Pointer => ErasedCallable::fromStateAndFn(state, callPyCallback::<FfiPointer>),
    FfiType::Struct(_) => return Err(CallError::Decode(
      "chillffi-py callback: struct return type is not supported".to_string()
    ))
  })
}

/// Плоское представление результата Python-вызова: только POD — всё
/// питоновское умирает под GIL, паника делается уже после его отпускания.
enum CallbackRepr
{
  Int(i128),
  Float(f64),
  Bool(bool),
  None
}

/// Результат Python → конкретный Output-тип (`FfiPrimitive`).
trait PyCallbackResult: Sized
{
  fn fromRepr(value: CallbackRepr) -> Result<Self, String>;
}

// todo desc
impl PyCallbackResult for ()
{
  fn fromRepr(_value: CallbackRepr) -> Result<Self, String>
  {
    Ok(())
  }
}

macro_rules! implCallbackResultInt
{
  ($ty:ty) =>
  {
    impl PyCallbackResult for $ty
    {
      fn fromRepr(value: CallbackRepr) -> Result<Self, String>
      {
        match value
        {
          CallbackRepr::Int(v) => <$ty>::try_from(v).map_err(|_|
            format!("callback returned {v}: out of range for {}", stringify!($ty))
          ),
          CallbackRepr::Bool(v) => Ok(if v { 1 } else { 0 }),
          CallbackRepr::Float(v) => Ok(v as $ty),
          CallbackRepr::None => Err("callback returned None, expected a number".to_string())
        }
      }
    }
  };
}

implCallbackResultInt!(i8);
implCallbackResultInt!(i16);
implCallbackResultInt!(i32);
implCallbackResultInt!(i64);
implCallbackResultInt!(isize);
implCallbackResultInt!(u8);
implCallbackResultInt!(u16);
implCallbackResultInt!(u32);
implCallbackResultInt!(u64);
implCallbackResultInt!(usize);

impl PyCallbackResult for f32
{
  fn fromRepr(value: CallbackRepr) -> Result<Self, String>
  {
    Ok(match value
    {
      CallbackRepr::Int(v) => v as f32,
      CallbackRepr::Float(v) => v as f32,
      CallbackRepr::Bool(v) => { if v { 1.0 } else { 0.0 } },
      CallbackRepr::None => return Err("callback returned None, expected f32".to_string())
    })
  }
}

impl PyCallbackResult for f64
{
  fn fromRepr(value: CallbackRepr) -> Result<Self, String>
  {
    Ok(match value
    {
      CallbackRepr::Int(v) => v as f64,
      CallbackRepr::Float(v) => v,
      CallbackRepr::Bool(v) => { if v { 1.0 } else { 0.0 } },
      CallbackRepr::None => return Err("callback returned None, expected f64".to_string())
    })
  }
}

impl PyCallbackResult for bool
{
  fn fromRepr(value: CallbackRepr) -> Result<Self, String>
  {
    Ok(match value
    {
      CallbackRepr::Bool(v) => v,
      CallbackRepr::Int(v) => v != 0,
      CallbackRepr::Float(v) => v != 0.0,
      CallbackRepr::None => return Err("callback returned None, expected bool".to_string())
    })
  }
}

impl PyCallbackResult for FfiPointer
{
  fn fromRepr(value: CallbackRepr) -> Result<Self, String>
  {
    match value
    {
      CallbackRepr::Int(v) => usize::try_from(v)
        .map(FfiPointer)
        .map_err(|_| format!("callback returned {v}: negative pointer")),
      CallbackRepr::None => Ok(FfiPointer(0)),
      _ => Err("callback returned non-integer, expected pointer".to_string())
    }
  }
}

/// Один C-аргумент → питоновский объект (под GIL клона).
/// Указатель остаётся адресом клона — тело разыменовывает его ctypes-ом.
fn callbackArgToPython(
  py: Python<'_>,
  args: &DynamicList,
  tag: u8,
  index: usize
) -> Result<Py<PyAny>, String>
{
  macro_rules! intArg
  {
    ($ty:ty) =>
    {
      {
        let v: $ty = args.get::<$ty>(index)
          .map_err(|e| format!("callback arg {index}: {e:?}"))?;
        v.into_pyobject(py).map_err(|e| format!("callback arg {index}: {e}"))?
          .into_any().unbind()
      }
    };
  }
  Ok(match tag
  {
    TAG_I8 => intArg!(i8),
    TAG_I16 => intArg!(i16),
    TAG_I32 => intArg!(i32),
    TAG_I64 => intArg!(i64),
    TAG_ISIZE => intArg!(isize),
    TAG_U8 => intArg!(u8),
    TAG_U16 => intArg!(u16),
    TAG_U32 => intArg!(u32),
    TAG_U64 => intArg!(u64),
    TAG_USIZE => intArg!(usize),
    TAG_F32 =>
    {
      let v: f32 = args.get::<f32>(index).map_err(|e| format!("callback arg {index}: {e:?}"))?;
      v.into_pyobject(py).map_err(|e| format!("callback arg {index}: {e}"))?.into_any().unbind()
    }
    TAG_F64 =>
    {
      let v: f64 = args.get::<f64>(index).map_err(|e| format!("callback arg {index}: {e:?}"))?;
      v.into_pyobject(py).map_err(|e| format!("callback arg {index}: {e}"))?.into_any().unbind()
    }
    TAG_BOOL =>
    {
      let v: bool = args.get::<bool>(index).map_err(|e| format!("callback arg {index}: {e:?}"))?;
      pyo3::types::PyBool::new(py, v).to_owned().into_any().unbind()
    }
    TAG_POINTER =>
    {
      let v: FfiPointer = args.get::<FfiPointer>(index).map_err(|e| format!("callback arg {index}: {e:?}"))?;
      v.0.into_pyobject(py).map_err(|e| format!("callback arg {index}: {e}"))?.into_any().unbind()
    }
    other => return Err(format!("callback arg {index}: unknown type tag {other}"))
  })
}

/// Исполнение питоновского тела в клоне: распаковка marshal/pickle,
/// сборка функции, вызов, плоский результат. Работает под GIL того же
/// интерпретатора, что поднял зиготу (`python -I -c "...chillffiRunZygote..."`):
/// ctypes отпустил GIL на время `chillffiRunZygote`, поэтому
/// `PyGILState_Ensure` трэмполина честно захватывает его обратно —
/// тот же шаблон, что у ctypes-callbacks.
fn runCallbackPython(
  py: Python<'_>,
  state: &PyCallbackState,
  args: &DynamicList
) -> Result<CallbackRepr, String>
{
  let result: Result<Result<CallbackRepr, String>, String> = std::panic::catch_unwind(
    std::panic::AssertUnwindSafe(||
    {
      let marshalMod: Bound<'_, PyModule> = py.import("marshal")
        .map_err(|e| format!("import marshal: {e}"))?;
      let pickleMod: Bound<'_, PyModule> = py.import("pickle")
        .map_err(|e| format!("import pickle: {e}"))?;
      let typesMod: Bound<'_, PyModule> = py.import("types")
        .map_err(|e| format!("import types: {e}"))?;

      // Код и окружение.
      let codeBytes: &[u8] = &state.code[..state.codeLen as usize];
      let codeObj: Bound<'_, PyAny> = marshalMod
        .call_method1("loads", (pyo3::types::PyBytes::new(py, codeBytes),))
        .map_err(|e| format!("marshal.loads: {e}"))?;
      let globalsDict: Bound<'_, PyDict> = PyDict::new(py);
      let builtins: Bound<'_, PyModule> = py.import("builtins")
        .map_err(|e| format!("import builtins: {e}"))?;
      globalsDict.set_item("__builtins__", builtins.getattr("__dict__").map_err(|e| format!("builtins dict: {e}"))?)
        .map_err(|e| format!("set __builtins__: {e}"))?;
      if state.globalsLen > 0
      {
        let snapshotBytes: &[u8] = &state.globals[..state.globalsLen as usize];
        let snapshot: Bound<'_, PyDict> = pickleMod
          .call_method1("loads", (PyBytes::new(py, snapshotBytes),))
          .map_err(|e| format!("pickle.loads globals: {e}"))?
          .cast_into()
          .map_err(|e| format!("globals snapshot is not a dict: {e}"))?;
        for (name, value) in snapshot.iter()
        {
          globalsDict.set_item(name, value).map_err(|e| format!("globals snapshot: {e}"))?;
        }
      }

      // Функция: FunctionType(code, globals, name=None, argdefs, closure=None).
      let defaultsObj: Py<PyAny> = if state.defaultsLen > 0
      {
        let defaultsBytes: &[u8] = &state.defaults[..state.defaultsLen as usize];
        pickleMod
          .call_method1("loads", (PyBytes::new(py, defaultsBytes),))
          .map_err(|e| format!("pickle.loads defaults: {e}"))?
          .unbind()
      } else {
        py.None()
      };
      let fnObj: Bound<'_, PyAny> = typesMod
        .getattr("FunctionType").map_err(|e| format!("FunctionType: {e}"))?
        .call1((codeObj, globalsDict, py.None(), defaultsObj, py.None()))
        .map_err(|e| format!("FunctionType: {e}"))?;

      // Аргументы C → Python.
      let mut pyArgs: Vec<Py<PyAny>> = Vec::with_capacity(state.argCount as usize);
      for (i, tag) in state.argTypes[..state.argCount as usize].iter().enumerate() {
        pyArgs.push(callbackArgToPython(py, args, *tag, i)?);
      }
      let argsTuple: Bound<'_, PyTuple> = PyTuple::new(py, pyArgs)
        .map_err(|e| format!("args tuple: {e}"))?;

      // Вызов.
      let outcome: Result<Bound<'_, PyAny>, PyErr> = fnObj.call1(argsTuple);
      match outcome
      {
        Ok(value) =>
        {
          let repr: CallbackRepr = if value.is_none() {
            CallbackRepr::None
          }
          else if value.is_instance_of::<pyo3::types::PyBool>() {
            CallbackRepr::Bool(value.extract::<bool>().map_err(|e| format!("bool extract: {e}"))?)
          }
          else if value.is_instance_of::<pyo3::types::PyFloat>() {
            CallbackRepr::Float(value.extract::<f64>().map_err(|e| format!("float extract: {e}"))?)
          }
          else {
            CallbackRepr::Int(value.extract::<i128>().map_err(|e| format!("int extract: {e}"))?)
          };
          Ok(repr)
        }
        Err(err) =>
        {
          // Исключение питоновского тела: traceback в stderr клона (виден
          // на терминале), затем ошибка уходит как CallbackPanicked →
          // `FFIError` на хосте — зеркально панике closure в Rust.
          let stderr: Result<Bound<'_, PyAny>, _> =
            py.import("sys").and_then(|sys| sys.getattr("stderr"));
          if let Ok(stderr) = stderr {
            let _ = stderr.call_method1(
              "write",
              (format!("chillffi-py callback python error: {err}\n"),)
            );
          }
          Err(format!("python callback failed: {err}"))
        }
      }
    })
  ).map_err(|payload| match payload.downcast::<String>()
  {
    Ok(message) => format!("callback panicked: {message}"),
    Err(_) => "callback panicked".to_string()
  });
  result?
}

/// Типизированный вход для `ErasedCallable::fromStateAndFn` — Output-тип
/// фиксируется в pyCallbackDecode по returnType. Паника ловится
/// `catch_unwind` в трэмполине `chillffi` → CallbackPanicked → `FFIError`.
fn callPyCallback<T: chillffi::ffi::types::primitive::FfiPrimitive + PyCallbackResult>(
  state: &PyCallbackState,
  args: &DynamicList
) -> T
{
  let outcome: Result<CallbackRepr, String> = Python::attach(|py| runCallbackPython(py, state, args));
  match outcome
  {
    Ok(repr) => T::fromRepr(repr).unwrap_or_else(|message| panic!("chillffi-py callback: {message}")),
    Err(message) => panic!("chillffi-py callback: {message}")
  }
}

/// Хэндл зарегистрированного колбэка — проброс `chillffi::Callback`:
/// кладётся в `.arg(...)` / прямой вызов, клон резолвит его в code pointer.
#[pyclass(name = "Callback")]
pub struct PyCallback
{
  // todo desc
  pub slot: Arc<ScopeSlot>,

  // todo desc
  pub handle: FfiCallback,

  // todo desc
  pub signature: String
}

#[pymethods]
impl PyCallback
{
  /// Сигнатура, под которой колбэк был зарегистрирован.
  fn signature(&self) -> &str
  {
    &self.signature
  }

  /// Колбэк пригоден к использованию: скоуп, в котором он зарегистрирован,
  /// ещё открыт (после выхода из `with` хэндл недействителен).
  fn registered(&self) -> bool
  {
    self.slot.isOpen()
  }

  // todo desc
  fn __repr__(&self) -> String
  {
    format!("<Callback '{}' id={:#x}>", self.signature, { self.handle.id() })
  }
}

// =================================================================================================
