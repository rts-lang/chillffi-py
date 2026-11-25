"""
errno — зеркало chillffi/examples/errno/main.rs.

Захват errno: на уровне вызова, скоупа и глобально (`chillffi.setGlobalReadErrno`).
Приоритет: вызов > скоуп > глобальный.

На Windows есть второй канал ошибок — `GetLastError()`, он захватывается под тем же
флагом `.errno()`, что и errno CRT, и читается через `Scope::lastOsError`.
Отдельного метода билдера нет: один флаг управляет обоими каналами. На Unix
`lastOsError()` всегда None.

C-помощник собирает `run.rs` (запуск примера) -> `examples/errno/liberrno.so`

Запуск: python3 examples/errno/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import platformExt  # noqa: E402

ErrnoDir: str = os.path.join(TestDir, "errno")

# ======================================================================================================================
# `.errno()` на одном вызове -> errno CRT через `scope.lastErrno()`.

def callErrno() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(ErrnoDir)
    liberrno: chillffi.Library = scope.load(platformExt("liberrno"))

    result: int = (
      liberrno.failWithErrno
      .arg(2)  # ENOENT
      .errno()
      .result("i32")
    )

    assert result == -1, f"failWithErrno != -1 ({result})"
    errno: int | None = scope.lastErrno()

  assert errno == 2, f"errno != 2 ({errno})"
  print("ok: call-level errno ->", errno)

# ======================================================================================================================
# Короткой записи `liberrno.failWithErrno(9)` больше нет: тип результата
# C-функции не выводится из значений аргументов, вызов билдера как функции —
# ошибка с подсказкой (см. `library.rs`); явный тип + захват errno даёт
# честные -1 и errno=9.

def callErrnoDirectCall() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(ErrnoDir)
    liberrno: chillffi.Library = scope.load(platformExt("liberrno"))

    # Короткая запись отклоняется — подсказка ведёт в цепочку.
    shortError: str | None = None
    try:
      liberrno.failWithErrno(9)   # EBADF
    except RuntimeError as error:
      shortError = str(error)

    # Цепочка: явный тип + захват errno — честное -1 и errno=9.
    typedResult: int = (
      liberrno.failWithErrno
      .arg(9)
      .errno()
      .result("i32")
    )
    typedErrno: int | None = scope.lastErrno()

  assert typedResult == -1, f"failWithErrno != -1 ({typedResult})"
  assert typedErrno == 9, f"errno != 9 ({typedErrno})"
  assert shortError and "короткая запись" in shortError, f"короткая запись не отклонена: {shortError}"
  print("ok: короткая запись отклонена ->", shortError[:56], "...")
  print(f"ok: errno chain-only -> errno={typedErrno}, Typed(i32)={typedResult}")

# ======================================================================================================================
# `setReadErrno(true)` на скоупе — каждый вызов в блоке захватывает errno,
# пока вызов не перебил флаг сам. Rust: `scope.setReadErrno(true)` +
# `Scope::lastErrno()`. Приоритет политики — как в `chillffi`
# (`resolveReadErrno`): per-call (`.errno()`/`.noErrno()`) > скоуп
# (`setReadErrno`) > глобальный дефолт (`chillffi.setGlobalReadErrno`,
# изначально `false`).

def scopeErrno() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(ErrnoDir)
    liberrno: chillffi.Library = scope.load(platformExt("liberrno"))

    scope.setReadErrno(True)

    # Скоуп-уровень: errno захватывается без `.errno()` в цепочке.
    result: int = (
      liberrno.failWithErrno
      .arg(2)  # ENOENT
      .result("i32")
    )
    scopeErrnoValue: int | None = scope.lastErrno()

    # Per-call `.noErrno()` перебивает скоуп: последний вызов не захватывал errno,
    # поэтому `lastErrno()` сбрасывается в None (семантика ядра: errno последнего вызова).
    resultNoErrno: int = (
      liberrno.failWithErrno
      .arg(9)  # EBADF
      .noErrno()
      .result("i32")
    )
    afterNoErrno: int | None = scope.lastErrno()

  assert result == -1, f"failWithErrno != -1 ({result})"
  assert scopeErrnoValue == 2, f"scope errno != 2 ({scopeErrnoValue})"
  assert resultNoErrno == -1, f"failWithErrno(.noErrno()) != -1 ({resultNoErrno})"
  assert afterNoErrno is None, f".noErrno() перебил скоуп — errno не захватывался: {afterNoErrno}"
  print(f"ok: scope-level errno -> errno={scopeErrnoValue} (без .errno()); "
        f".noErrno() перебил скоуп -> lastErrno()={afterNoErrno} (последний вызов не захватывал)")

# ======================================================================================================================
# Глобальный `setGlobalReadErrno(true)` — нижний уровень политики.
# Rust: `chillffi::errnoPolicy::setGlobalReadErrno(true)`;
# `chillffi.setGlobalReadErrno(enabled)` — глобальный дефолт, настройка
# процесса (как в ядре). Голый вызов — без `.errno()`/`.noErrno()` и скоупа
# без `setReadErrno` — наследует его; оба верхних уровня перебивают
# глобальный в обе стороны.

def globalErrno() -> None:
  # Дефолт выключен: вызов без переопределений errno не захватывает.
  with chillffi.Scope() as scope:
    scope.addSearchPath(ErrnoDir)
    liberrno: chillffi.Library = scope.load(platformExt("liberrno"))

    beforeResult: int = (
      liberrno.failWithErrno
      .arg(2)  # ENOENT
      .result("i32")
    )
    errnoBefore: int | None = scope.lastErrno()

  assert beforeResult == -1, f"failWithErrno != -1 ({beforeResult})"
  assert errnoBefore is None, \
    f"дефолт выключен — errno не должен захватываться: {errnoBefore}"
  print("ok: global default off -> lastErrno() =", errnoBefore)

  # Глобальный дефолт включён: тот же голый вызов захватывает errno.
  chillffi.setGlobalReadErrno(True)

  with chillffi.Scope() as scope:
    scope.addSearchPath(ErrnoDir)
    liberrno: chillffi.Library = scope.load(platformExt("liberrno"))

    # Голый вызов (без .errno() и без скоупа) — наследует глобальный дефолт.
    globalResult: int = (
      liberrno.failWithErrno
      .arg(9)  # EBADF
      .result("i32")
    )
    errnoGlobal: int | None = scope.lastErrno()

    # Per-call `.noErrno()` перебивает глобальный: последний вызов не захватывал.
    noErrnoResult: int = (
      liberrno.failWithErrno
      .arg(2)  # ENOENT
      .noErrno()
      .result("i32")
    )
    errnoAfterNoErrno: int | None = scope.lastErrno()

    # Скоуп-настройка тоже перебивает глобальный — в обе стороны.
    scope.setReadErrno(False)
    scopeFalseResult: int = (
      liberrno.failWithErrno
      .arg(2)  # ENOENT
      .result("i32")
    )
    errnoAfterScopeFalse: int | None = scope.lastErrno()

    scope.setReadErrno(True)
    scopeTrueResult: int = (
      liberrno.failWithErrno
      .arg(9)  # EBADF
      .result("i32")
    )
    errnoAfterScopeTrue: int | None = scope.lastErrno()

  assert globalResult == -1, f"failWithErrno != -1 ({globalResult})"
  assert errnoGlobal == 9, f"глобальный дефолт не сработал: {errnoGlobal}"
  assert noErrnoResult == -1, f"failWithErrno(.noErrno()) != -1 ({noErrnoResult})"
  assert errnoAfterNoErrno is None, \
    f".noErrno() перебил глобальный — errno не захватывался: {errnoAfterNoErrno}"
  assert scopeFalseResult == -1, f"failWithErrno != -1 ({scopeFalseResult})"
  assert errnoAfterScopeFalse is None, \
    f"scope.setReadErrno(False) перебил глобальный: {errnoAfterScopeFalse}"
  assert errnoAfterScopeTrue == 9, \
    f"scope.setReadErrno(True) должен захватывать: {errnoAfterScopeTrue}"
  print(f"ok: global-level errno -> errno={errnoGlobal} (голый вызов); ")
  print(f"    перебивают в обе стороны: .noErrno() -> {errnoAfterNoErrno}, "
        f"scope(False) -> {errnoAfterScopeFalse}, scope(True) -> {errnoAfterScopeTrue}")

  # Глобальная настройка — уровня процесса: вернуть дефолт для последующих блоков.
  chillffi.setGlobalReadErrno(False)

  with chillffi.Scope() as scope:
    scope.addSearchPath(ErrnoDir)
    liberrno: chillffi.Library = scope.load(platformExt("liberrno"))

    afterResult: int = (
      liberrno.failWithErrno
      .arg(2)  # ENOENT
      .result("i32")
    )
    errnoAfter: int | None = scope.lastErrno()

  assert afterResult == -1, f"failWithErrno != -1 ({afterResult})"
  assert errnoAfter is None, \
    f"после возврата дефолта errno не должен захватываться: {errnoAfter}"
  print("ok: global default restored -> lastErrno() =", errnoAfter)

# ======================================================================================================================
# Windows: тот же `.errno()` захватывает `GetLastError` -> `Scope::lastOsError`.

# Windows: тот же `.errno()` захватывает `GetLastError` -> `Scope::lastOsError`.
# Rust: `Scope::lastOsError()` (канал `GetLastError`, только Windows).
# Не поддерживается: `Scope.lastOsError()` в Python API нет (есть только
# `lastErrno()` — канал CRT). Нужен проброс `Scope::lastOsError` из
# `chillffi/src/ffi/scope.rs:394`.

def callOsError() -> None:
  print("SKIP: Scope.lastOsError() не выставлен в Python API — нужен проброс из chillffi/src/ffi/scope.rs")

# ======================================================================================================================

def main() -> None:
  callErrno()
  callErrnoDirectCall()
  scopeErrno()
  globalErrno()
  callOsError()

if __name__ == "__main__":
  main()

# ======================================================================================================================
