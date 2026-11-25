// chillffi-py — PyO3-обвязка над `chillffi`: `cdylib`, питон-модуль `chillffi`.
// Процессы (зигота, клон) создаёт само ядро; хост без своего `main()` один раз
// сообщает `chillffi`, чем запускать зиготу (`setZygoteCommand` — см. `setup`).
// =================================================================================================

// Модули: реализация разложена по тематике.
//
//   scope.rs    — удерживаемый scope, `Scope` и движок структур (`allocStruct`,
//                 `readDynamicStruct`/`writeDynamicStruct`, `readMemory`);
//   library.rs  — `Library` и `CallBuilder`;
//   args.rs     — типизированные аргументы вызова;
//   bridge.rs   — вызов символа и выбор типа результата;
//   callback.rs — питоновская функция как C function pointer;
//   memory.rs   — блок памяти в куче клона + `readStruct`/`writeStruct`;
//   structs.rs  — `Type` и `StructValue`: движок структур (зеркало
//                 `chillffi/src/ffi/types`);
//   error.rs    — `FFIError` → `PyErr`;
//   errnoPolicy.rs — глобальный уровень errno-политики (`setGlobalReadErrno`);
//   pytypes.rs  — питоновские типы FFI (зеркало `Type` из `chillffi`) + интероп ctypes;
//   pathResolver.rs — глобальный резолвер путей (`addGlobalSearchPath`);
//   zygote.rs   — entry зиготы.

// =================================================================================================
mod args;
mod bridge;
mod callback;
mod errnoPolicy;
mod error;
mod library;
mod memory;
mod pathResolver;
mod pytypes;
mod scope;
mod structs;
mod zygote;
// =================================================================================================
use crate::callback::PyCallback;
use crate::errnoPolicy::setGlobalReadErrno;
use crate::library::{PyCall, PyLib};
use crate::memory::PyMemory;
use crate::pathResolver::addGlobalSearchPath;
use crate::pytypes::{
  TypedBool, TypedCString, TypedF32, TypedF64, TypedI16, TypedI32, TypedI64, TypedI8, TypedIsize,
  TypedPointer, TypedRawString, TypedString, TypedU16, TypedU32, TypedU64, TypedU8, TypedUsize,
};
use crate::scope::Scope;
use crate::structs::{PyFfiType, PyStructValue};
pub use crate::zygote::chillffiRunZygote;
use pyo3::prelude::*;
use pyo3::types::PyModule;
// =================================================================================================
// setup

/// Едиственная задача `setup` — сообщить `chillffi`, чем поднимать Main Zygote
/// (host — интерпретатор, у `chillffi` нет своего `main()`/`ctor` здесь).
/// Сама зигота поднимается лениво самим `chillffi` на первой FFI-операции;
/// одна на процесс; из Python её не видно и не достать.
#[pymodule(name = "chillffi")]
fn chillffiPy(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()>
{
  // Зигота = этот же интерпретатор в изолированном режиме: он dlopen-ит
  // этот же `.so` и входит в цикл зиготы. Один и тот же `.so` у хоста и
  // у зиготы — обязателен для колбэков (`moduleBase()` + `offset`).
  let soFile: Option<String> = module.getattr("__file__").ok().and_then(|v| v.extract().ok());
  let exe: Option<String> = py.import("sys").ok()
    .and_then(|sys| sys.getattr("executable").ok())
    .and_then(|v| v.extract().ok());

  if let (Some(soPath), Some(exe)) = (soFile, exe)
  {
    let bootstrap: String = format!(
      "import ctypes,sys; ctypes.CDLL({soPath:?}).chillffiRunZygote(sys.argv[1].encode())"
    );
    chillffi::setZygoteCommand(
      std::ffi::OsString::from(exe),
      vec![
        std::ffi::OsString::from("-I"),
        std::ffi::OsString::from("-c"),
        std::ffi::OsString::from(bootstrap)
      ]
    );
    // false (уже настроено, повторный `import`) — молча игнорируем.
  }

  module.add("__version__", env!("CARGO_PKG_VERSION"))?;

  // Настройки уровня модуля: резолв пути и errno-политика применяются
  // на самих вызовах (`Scope::load` ядра / `PyCall::finish`).
  module.add_function(wrap_pyfunction!(addGlobalSearchPath, module)?)?;
  module.add_function(wrap_pyfunction!(setGlobalReadErrno, module)?)?;

  module.add_class::<Scope>()?;
  module.add_class::<PyLib>()?;
  module.add_class::<PyCall>()?;
  module.add_class::<PyCallback>()?;
  module.add_class::<PyMemory>()?;

  // Движок структур — зеркало `chillffi/src/ffi/types` (`structs.rs`):
  // `Type` (форма раскладки) и `StructValue` (структура по значению).
  module.add_class::<PyFfiType>()?;
  module.add_class::<PyStructValue>()?;

  // Типы FFI — зеркало `chillffi::ffi::types::Type` (`pytypes.rs`):
  // числовые, `Pointer` и строковые; принимают ctypes-значения (`c_int32` и т.д.).
  module.add_class::<TypedI8>()?;
  module.add_class::<TypedI16>()?;
  module.add_class::<TypedI32>()?;
  module.add_class::<TypedI64>()?;
  module.add_class::<TypedIsize>()?;
  module.add_class::<TypedU8>()?;
  module.add_class::<TypedU16>()?;
  module.add_class::<TypedU32>()?;
  module.add_class::<TypedU64>()?;
  module.add_class::<TypedUsize>()?;
  module.add_class::<TypedF32>()?;
  module.add_class::<TypedF64>()?;
  module.add_class::<TypedBool>()?;
  module.add_class::<TypedPointer>()?;
  module.add_class::<TypedCString>()?;
  module.add_class::<TypedRawString>()?;
  module.add_class::<TypedString>()?;
  Ok(())
}

// =================================================================================================
