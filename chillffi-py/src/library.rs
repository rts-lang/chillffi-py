use crate::args::{FfiArgValue, pyToFfiArg};
use crate::bridge::{RetVal, callExternalStructWithScope, callExternalWithScope, callPointerWithScope, retValToPython};
use crate::errnoPolicy::globalReadErrno;
use crate::error::toPyErr;
use crate::pytypes::expectToken;
use crate::scope::ScopeSlot;
use crate::structs::{shapeFromPy, structValueToPython};
use chillffi::ffi::errors::FFIError;
use chillffi::ffi::library::Library;
use chillffi::ffi::scope::{FFIScope, Scope as ChillScope};
use chillffi::ffi::types::Type as FfiType;
use pyo3::exceptions::{PyAttributeError, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
// =================================================================================================
// library.rs — питоновские обёртки над библиотекой и её символами:
// `PyLib` и цепочка вызова `PyCall`.
// =================================================================================================
// PyLib (python): проброс методов library

/// Хэндл библиотеки внутри скоупа: хранит (скоуп, путь),
/// а все методы пробрасываются в `chillffi` на вызове.
/// Символы ищутся динамически: `lib.sqrt` → `__getattr__("sqrt")` → `CallBuilder`.
#[pyclass(name = "Library")]
pub struct PyLib
{
  // todo desc
  pub slot: Arc<ScopeSlot>,

  // todo desc
  pub path: String,

  /// `Library::unload` забирает `self` — после него хэндл использовать нельзя.
  /// Флаг общий с `CallBuilder`, полученными от этой библиотеки.
  pub unloaded: Arc<AtomicBool>
}

#[pymethods]
impl PyLib
{
  /// Резолвнутый путь библиотеки.
  fn path(&self) -> &str
  {
    &self.path
  }

  /// Проброс `Library::unload`. В Rust метод забирает `self` и хэндл после нельзя
  /// использовать; здесь после `unload()` любое обращение к этой `Library`, к её
  /// `CallBuilder` и повторный `unload()` — `RuntimeError` (`LibraryNotFound`).
  ///
  /// Хэндла `chillffi` между вызовами нет (библиотека грузится на каждый вызов),
  /// поэтому `Library::unload` зовётся на свежем хэндле; ограничение держит
  /// флаг `unloaded`. Из-за атрибутного старта C-символ с именем `unload`
  /// (как у плагинных библиотек) из Python недостижим — обращение затенено
  /// этим методом.
  fn unload(&self, py: Python<'_>) -> PyResult<()>
  {
    if !self.slot.isOpen() {
      return Err(PyRuntimeError::new_err("scope is closed"));
    }
    if self.unloaded.swap(true, Ordering::SeqCst) {
      return Err(unloadedError(&self.path));
    }
    let path: String = self.path.clone();
    let slot: Arc<ScopeSlot> = self.slot.clone();
    py.detach(move ||
    {
      slot.withScope(move |ffiScope: &FFIScope|
      {
        let scope: ChillScope<'_> = ffiScope.scope();
        let lib: Library<'_> = scope.load(&path)?;
        lib.unload()
      })
    }).map_err(toPyErr)?.map_err(toPyErr)
  }

  /// Символ библиотеки — старт цепочки: `lib.sqrt.arg(16.0).result("f64")`.
  /// Само обращение к атрибуту символ не резолвит и не запускает: dlopen/dlsym
  /// и вызов происходят на финализаторе в клоне, ошибки пути/символа/типа —
  /// там же. Имя из переменной достаётся штатным `getattr(lib, symbolName)` —
  /// это тот же `__getattr__`. Имена, затенённые методами обвязки (`path`,
  /// `unload`), из Python недостижимы.
  fn __getattr__(&self, name: &str) -> PyResult<PyCall>
  {
    if !self.slot.isOpen() {
      return Err(PyRuntimeError::new_err("scope is closed"));
    }
    self.checkLoaded()?;
    // Python-протоколы probing'ают приватные имена (`__deepcopy__`,
    // `_ipython_canary_...`): билдер вместо AttributeError их ломает.
    // Настоящие символы с `_` (`_exit`, `_environ`) недоступны вместе
    // с именами, затенёнными методами обвязки.
    if name.starts_with('_')
    {
      return Err(PyAttributeError::new_err(format!(
        "Library has no attribute '{name}': symbols starting with '_' and \
         names shadowed by Library methods are unreachable"
      )));
    }
    Ok(PyCall::named(self.slot.clone(), self.path.clone(), name.to_string(), self.unloaded.clone()))
  }
}

impl PyLib
{
  // todo desc
  fn checkLoaded(&self) -> PyResult<()>
  {
    if self.unloaded.load(Ordering::SeqCst) {
      return Err(unloadedError(&self.path));
    }
    Ok(())
  }
}

/// Использование библиотеки после `unload()` — тот же `FFIError`, что `chillffi`
/// отдаёт для библиотеки, которой нет в реестре.
pub fn unloadedError(path: &str) -> PyErr
{
  toPyErr(FFIError::LibraryNotFound { libraryPath: path.to_string() })
}

// =================================================================================================
// CallBuilder (python): единственный способ вызвать — цепочка a.b().c()

/// Цепочка вызова — зеркало `CallBuilder`/`VariadicCallBuilder` из `chillffi`:
/// `.arg()` / `.variadic()` / `.errno()` / `.noErrno()` и финализатор
/// `.result(...)` — единственная точка запуска.
///
/// Старт цепочки: `lib.имя` — символ библиотеки, `scope.pointer(адрес)` —
/// вызов по адресу из клона. Тип результата финализатор объявляет явно —
/// из значений аргументов он не выводится (подробности — на `result` ниже):
///
/// ```python
/// lib.abs.arg(-5).result("i32")            # токен: как .result::<i32>() в Rust
/// lib.abs.arg(-5).result(chillffi.I32)     # класс нашего типа — то же
/// lib.abs.arg(-5).result(ctypes.c_int32)   # класс ctypes — то же
/// lib.srand.arg(42).result()               # пусто = void: запустить и выбросить
/// lib.failWithErrno.arg(2).errno().result("i32")   # errno → scope.lastErrno()
/// libc.printf.arg(fmt).variadic().arg(42).result("i32")  # varargs
/// scope.pointer(addr).arg(x).result("i32") # вызов по адресу из клона
/// scope.pointer(addr).arg(x).result()      # ~ Scope::callvPointer
/// ```
#[pyclass(name = "CallBuilder")]
pub struct PyCall
{
  /// Скоуп, в котором живёт вызов.
  slot: Arc<ScopeSlot>,

  /// Флаг `unload()` библиотеки-владельца. У `Pointer`-цели библиотеки нет —
  /// сюда кладётся свежий флаг (всегда false): адрес не привязан к библиотеке.
  unloaded: Arc<AtomicBool>,

  /// Цель вызова: символ библиотеки или адрес в клоне.
  target: Target,

  /// Уже приклеенные аргументы.
  args: Vec<FfiArgValue>,

  /// `Some(n)` — `.variadic()` был вызван, первые `n` аргументов на момент
  /// вызова заморожены как фиксированные (потребляющий `.variadic()` ядра).
  fixedArgs: Option<usize>,

  /// errno-режим: `None` — решить по политике: скоуп (`Scope::setReadErrno`),
  /// затем глобальный дефолт (`chillffi.setGlobalReadErrno`, `errnoPolicy.rs`).
  errno: Option<bool>
}

/// Цель вызова билдера.
#[derive(Clone)]
enum Target
{
  /// Символ библиотеки: dlopen/dlsym на стороне клона (`Library::call`).
  Named { path: String, name: String },

  /// Адрес в пространстве клона (`Scope::callPointer`): вернулся из
  /// предыдущего вызова C (`signal()`, `calloc`) или `Memory.address()`.
  Pointer { address: usize }
}

#[pymethods]
impl PyCall
{
  /// Имя вызываемого символа (только у символ-цели).
  fn name(&self) -> PyResult<String>
  {
    match &self.target
    {
      Target::Named { name, .. } => Ok(name.clone()),
      Target::Pointer { .. } => Err(PyRuntimeError::new_err(
        "this CallBuilder was started by scope.pointer(addr): it calls an address, not a named symbol"
      ))
    }
  }

  /// Один аргумент, конвертация как у прямого вызова. Цепочка.
  fn arg(mut slf: PyRefMut<'_, Self>, object: Bound<'_, PyAny>) -> PyResult<Py<Self>>
  {
    if !slf.slot.isOpen() {
      return Err(PyRuntimeError::new_err("scope is closed"));
    }
    if slf.unloaded.load(Ordering::SeqCst) {
      return Err(toPyErr(slf.unloadedErr()));
    }
    slf.args.push(pyToFfiArg(object)?);
    Ok(slf.into())
  }

  /// Граница фиксированных аргументов заморожена здесь: все последующие
  /// `.arg()` уходят в `...`. Float в `...` корректны (`ffi_prep_cif_var`);
  /// C применяет default argument promotions в вариативной части
  /// (f32 → double, малые целые → int) — передавай уже повышенные типы.
  /// Только для символ-целей: вариативной формы
  /// у `Scope::callPointer` в `chillffi` нет.
  fn variadic(mut slf: PyRefMut<'_, Self>) -> PyResult<Py<Self>>
  {
    if !slf.slot.isOpen() {
      return Err(PyRuntimeError::new_err("scope is closed"));
    }
    if matches!(slf.target, Target::Pointer { .. })
    {
      return Err(PyRuntimeError::new_err(
        ".variadic() is only for symbol calls: chillffi has no variadic form of Scope::callPointer"
      ));
    }
    if slf.args.is_empty() {
      return Err(PyRuntimeError::new_err(
        ".variadic() with no fixed arguments: a C variadic function has at least one fixed argument"
      ));
    }
    slf.fixedArgs = Some(slf.args.len());
    Ok(slf.into())
  }

  /// Принудительно читать errno после вызова (см. `Scope::lastErrno`).
  fn errno(mut slf: PyRefMut<'_, Self>) -> Py<Self>
  {
    slf.errno = Some(true);
    slf.into()
  }

  /// Принудительно не читать errno.
  fn noErrno(mut slf: PyRefMut<'_, Self>) -> Py<Self>
  {
    slf.errno = Some(false);
    slf.into()
  }

  /// Финализация: запускает вызов и объявляет, чего ждать от результата —
  /// аналог `.result::<T>()` в Rust.
  ///
  /// С аргументом — тип результата объявлен явно: строковый токен
  /// (`.result("i32")`), класс нашего типа (`.result(chillffi.I32)`) или
  /// класс ctypes (`.result(ctypes.c_int32)`) — всё сводится в один токен
  /// диспетчера.
  ///
  /// Без аргумента — void: запустить и выбросить результат
  /// (`.result("void")` — то же).
  ///
  /// Infer нет ни в какой форме: регистр возврата C ABI позволяет читать
  /// одни и те же байты под любым типом, поэтому тип не выводится из
  /// значений аргументов — `strtod` с не-float аргументами дал бы 0 вместо
  /// 2.5, `sqrtf(16.0)` — мусор вместо 4.0, отрицательный `i32` —
  /// 4294967295 вместо -1.
  #[pyo3(signature = (expect = None))]
  fn result(&self, py: Python<'_>, expect: Option<&Bound<'_, PyAny>>) -> PyResult<Py<PyAny>>
  {
    let expect: String = match expect
    {
      Some(token) => expectToken(token)?,
      None => "void".to_string()
    };
    let outcome: Result<RetVal, FFIError> = self.finish(py, expect);
    retValToPython(py, outcome)
  }

  /// Билдер нельзя вызвать как функцию: короткой записи `lib.name(...)` нет.
  /// Ошибка подсказывает перейти на цепочку: тип результата в скобках не
  /// угадывается, а varargs/errno выражаются только в цепочке.
  #[pyo3(signature = (*_args))]
  fn __call__(&self, _args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>>
  {
    // Границы скоупа/выгрузки — первыми: ошибка обвязки не должна
    // маскироваться подсказкой про цепочку.
    if !self.slot.isOpen() {
      return Err(PyRuntimeError::new_err("scope is closed"));
    }
    if self.unloaded.load(Ordering::SeqCst) {
      return Err(toPyErr(self.unloadedErr()));
    }

    Err(PyRuntimeError::new_err(
      "короткая запись lib.name(...) убрана: тип результата C-функции нельзя вывести \
       из значений аргументов (strtod с не-float аргументами вернул бы 0 вместо 2.5, \
       sqrtf(16.0) — мусор вместо 4.0, отрицательный i32 — 4294967295 вместо -1). \
       Объяви тип финализатором: lib.sqrt.arg(16.0).result(\"f64\"); \
       пустой .result() — void (запустить и выбросить); errno — .errno().result(...); \
       varargs — .arg(fmt).variadic().arg(...)"
    ))
  }

  /// Финализация структурой по значению: запустить вызов и вернуть поля
  /// как `chillffi.StructValue` — аналог `.resultStruct(&[Type])` в Rust.
  /// Форма (в том числе вложенная — `Type.Structure([...])`) обязательна:
  /// возвращаемый буфер нетипизирован, пока его не прочитают.
  ///
  /// ```python
  /// out = lib.pointTranslate.arg(point).arg(10).arg(20) \
  ///   .resultStruct([chillffi.Type.I32, chillffi.Type.I32])
  /// out.get(0)  # → 11
  /// ```
  fn resultStruct(&self, py: Python<'_>, fields: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>>
  {
    let shape: Vec<FfiType> = shapeFromPy(fields)?;
    let structValue = self.finishStruct(py, shape.clone()).map_err(toPyErr)?;
    structValueToPython(py, &structValue, &shape)
  }
}

impl PyCall
{
  /// Старт цепочки по символу библиотеки (`lib.имя` → `__getattr__`).
  fn named(slot: Arc<ScopeSlot>, path: String, name: String, unloaded: Arc<AtomicBool>) -> Self
  {
    PyCall
    {
      slot,
      unloaded,
      target: Target::Named { path, name },
      args: Vec::new(),
      fixedArgs: None,
      errno: None
    }
  }

  /// Старт цепочки по адресу в клоне (`scope.pointer(адрес)`).
  pub(crate) fn pointer(slot: Arc<ScopeSlot>, address: usize) -> Self
  {
    PyCall
    {
      slot,
      unloaded: Arc::new(AtomicBool::new(false)),
      target: Target::Pointer { address },
      args: Vec::new(),
      fixedArgs: None,
      errno: None
    }
  }

  /// Библиотечный путь для ошибок `unload()` (у `Pointer`-цели его нет —
  /// её свежий флаг никогда не поднимается).
  fn libPath(&self) -> Option<&str>
  {
    match &self.target
    {
      Target::Named { path, .. } => Some(path),
      Target::Pointer { .. } => None
    }
  }

  /// Ошибка «библиотека выгружена» для цели билдера.
  fn unloadedErr(&self) -> FFIError
  {
    FFIError::LibraryNotFound
    {
      libraryPath: self.libPath().expect("unloaded flag is shared with the library").to_string()
    }
  }

  /// Общая финализация: проверки → клонирование → detach → bridge.
  fn finish(&self, py: Python<'_>, expect: String) -> Result<RetVal, FFIError>
  {
    self.prepared()?;

    let (slot, target) = (self.slot.clone(), self.target.clone());
    let args: Vec<FfiArgValue> = self.args.clone();
    let fixedArgs: Option<usize> = self.fixedArgs;
    // errno-политика: per-call → скоуп → глобальный дефолт
    // (`chillffi.setGlobalReadErrno`, `errnoPolicy.rs`): явный `.noErrno()`
    // не перебивается глобальным, голый вызов наследует дефолт.
    let errno: bool = self.errno.or_else(|| slot.readErrno()).unwrap_or_else(globalReadErrno);

    // GIL отпускается на время IPC-вызова: клон — отдельный процесс.
    py.detach(move || match target
    {
      Target::Named { path, name } =>
      {
        callExternalWithScope(&slot, &path, &name, args, fixedArgs, &expect, errno)
      }
      Target::Pointer { address } =>
      {
        callPointerWithScope(&slot, address, args, &expect, errno)
      }
    })
  }

  /// Финализация структурой по значению (`.resultStruct`): та же подготовка,
  /// другой bridge-вызов. Форма возвращаемого буфера объявлена вызывающим
  /// (`fields`).
  fn finishStruct(
    &self,
    py: Python<'_>,
    fields: Vec<FfiType>
  ) -> Result<chillffi::ffi::types::primitive::StructValue, FFIError>
  {
    self.prepared()?;

    let (slot, target) = (self.slot.clone(), self.target.clone());
    let args: Vec<FfiArgValue> = self.args.clone();
    let fixedArgs: Option<usize> = self.fixedArgs;
    // errno-политика — как в `finish` выше: per-call → скоуп → глобальный дефолт.
    let errno: bool = self.errno.or_else(|| slot.readErrno()).unwrap_or_else(globalReadErrno);

    py.detach(move || match target
    {
      Target::Named { path, name } =>
      {
        callExternalStructWithScope(&slot, &path, &name, args, fixedArgs, errno, fields)
      }
      // Формы возврата структуры у `Scope::callPointer` в `chillffi` нет —
      // только финализаторы билдера.
      Target::Pointer { .. } => Err(FFIError::Other(
        "Scope::callPointer has no struct-return form in chillffi: call through a library symbol (.resultStruct) or read the struct via Memory".to_string()
      ))
    })
  }

  /// Общие проверки перед запуском вызова: открытый скоуп, живая библиотека,
  /// граница variadic (после `.variadic()` нужен хотя бы один аргумент в `...`).
  fn prepared(&self) -> Result<(), FFIError>
  {
    if !self.slot.isOpen() {
      return Err(FFIError::Other("scope is closed".to_string()));
    }
    if self.unloaded.load(Ordering::SeqCst) {
      return Err(self.unloadedErr());
    }
    if let Some(fixed) = self.fixedArgs
    {
      if self.args.len() <= fixed
      {
        return Err(FFIError::Other(format!(
          "variadic call needs at least {} fixed argument(s) plus one variadic, got {}",
          fixed,
          self.args.len()
        )));
      }
    }
    Ok(())
  }
}

// =================================================================================================
