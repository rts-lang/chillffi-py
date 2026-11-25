use pyo3::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
// =================================================================================================
// errnoPolicy.rs — глобальный уровень errno-политики
// (зеркало `chillffi/src/errnoPolicy.rs`).
//
// Приоритет уровней: per-call (`.errno()`/`.noErrno()`, `library.rs`) >
// скоуп (`scope.setReadErrno`, `scope.rs`) > глобальный дефолт (этот файл).
// =================================================================================================

/// Глобальный дефолт errno-политики: `false`, пока
/// `chillffi.setGlobalReadErrno(True)` не включит.
static GlobalReadErrno: AtomicBool = AtomicBool::new(false);

/// Чтение глобального уровня для резолва политики (`PyCall::finish`) —
/// аналог `globalReadErrno()` ядра.
pub(crate) fn globalReadErrno() -> bool
{
  GlobalReadErrno.load(Ordering::Relaxed)
}

/// Проброс `chillffi::errnoPolicy::setGlobalReadErrno` — глобальный дефолт
/// errno-политики (нижний уровень приоритета, действует на все скоупы и
/// все потоки хоста).
///
/// С включённым дефолтом каждый вызов без своего `.errno()`/`.noErrno()` и
/// без `scope.setReadErrno(...)` захватывает errno — читается
/// `scope.lastErrno()`. Оба верхних уровня перебивают глобальный в обе
/// стороны (`.noErrno()` и `scope.setReadErrno(False)` отключают захват
/// даже при включённом глобальном).
///
/// ```python
/// chillffi.setGlobalReadErrno(True)
/// with chillffi.Scope() as scope:
///     lib = scope.load("liberrno.so")
///     lib.failWithErrno.arg(9).result("i32")   # без .errno() — дефолт
///     scope.lastErrno()                        # 9 (EBADF)
/// ```
///
/// Настройка процесса, а не скоупа: вернуть `False` — для всех скоупов.
/// Локальные аналоги — `scope.setReadErrno(enabled)` (блок) и
/// `.errno()`/`.noErrno()` в цепочке (один вызов).
#[pyfunction]
pub fn setGlobalReadErrno(enabled: bool) -> ()
{
  GlobalReadErrno.store(enabled, Ordering::Relaxed);
}

// =================================================================================================
