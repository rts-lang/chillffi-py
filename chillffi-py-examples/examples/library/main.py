"""
library — зеркало chillffi/examples/library/main.rs.

`Library`: load, path, unload и ошибки загрузки/поиска символа.

Запуск: python3 examples/library/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import LibmPath  # noqa: E402

# ======================================================================================================================
# `load` только запоминает путь; `path()` возвращает его как есть.

def loadCallAndPath() -> None:
  with chillffi.Scope() as scope:
    libm: chillffi.Library = scope.load(LibmPath)
    path: str = libm.path()
    result: float = libm.sqrt.arg(16.0).result("f64")

  assert path == LibmPath, f"path() != {LibmPath} ({path})"
  assert abs(result - 4.0) < 1e-12
  print(f"ok: loaded '{path}', sqrt(16.0) = {result}")

# ======================================================================================================================
# Явный `unload` — проброс `Library::unload`: хэндл забирается, дальше им пользоваться нельзя.
# Rust: `libm.unload()` забирает `self` — дальше `libm` это ошибка компиляции.
# В Python компилятора нет, то же самое — `RuntimeError` на любом обращении.

def explicitUnload() -> None:
  def assertUnloaded(what, action) -> None:
    try:
      action()
    except RuntimeError as e:
      assert "LibraryNotFound" in str(e), f"{what}: unexpected error: {e}"
      return
    raise SystemExit(f"{what}: использование выгруженной библиотеки НЕ упало — плохо")

  with chillffi.Scope() as scope:
    libm: chillffi.Library = scope.load(LibmPath)
    result: float = libm.sqrt.arg(9.0).result("f64")
    assert abs(result - 3.0) < 1e-12

    # Билдеры, полученные до `unload`, тоже не переживают хэндл: флаг общий.
    sqrtBuilder = libm.sqrt
    pending = libm.sqrt.arg(4.0)

    libm.unload()

    assertUnloaded("билдер до unload", lambda: sqrtBuilder.arg(16.0).result("f64"))
    assertUnloaded("__getattr__", lambda: libm.sqrt)
    assertUnloaded("CallBuilder __call__", lambda: sqrtBuilder(4.0))
    assertUnloaded("незакрытая цепочка", lambda: pending.result("f64"))
    assertUnloaded("повторный unload()", lambda: libm.unload())

    # Другой хэндл той же библиотеки в том же скоупе не затронут.
    other: chillffi.Library = scope.load(LibmPath)
    assert abs(other.sqrt.arg(16.0).result("f64") - 4.0) < 1e-12

  print("ok: explicit unload()")

# ======================================================================================================================
# Без явного `unload` библиотека освобождается вместе со скоупом.

def automaticDropOnScopeExit() -> None:
  with chillffi.Scope() as scope:
    libm: chillffi.Library = scope.load(LibmPath)
    result: float = libm.sqrt.arg(25.0).result("f64")
    assert abs(result - 5.0) < 1e-12

  print("ok: automatic drop on scope exit")

# ======================================================================================================================
# Плохой путь: `load` проходит (`dlopen` в клоне ленивый), ошибка приходит на первом вызове.

def libraryLoadFailed() -> None:
  with chillffi.Scope() as scope:
    bogus: chillffi.Library = scope.load("libTotallyDoesNotExist9000")
    try:
      bogus.whatever.result()
      raise SystemExit("вызов несуществующей библиотеки НЕ упал — плохо")
    except RuntimeError as e:
      assert "LibraryLoadFailed" in str(e), f"unexpected error: {e}"
      print("ok: LibraryLoadFailed —", str(e)[:110])

# ======================================================================================================================
# Библиотека есть, символа нет.

def symbolNotFound() -> None:
  with chillffi.Scope() as scope:
    libm: chillffi.Library = scope.load(LibmPath)
    try:
      libm.thisSymbolDoesNotExistAnywhere.result()
      raise SystemExit("вызов несуществующего символа НЕ упал — плохо")
    except RuntimeError as e:
      assert "SymbolNotFound" in str(e), f"unexpected error: {e}"
      print("ok: SymbolNotFound —", str(e)[:110])

# ======================================================================================================================

def main() -> None:
  loadCallAndPath()
  explicitUnload()
  automaticDropOnScopeExit()
  libraryLoadFailed()
  symbolNotFound()

if __name__ == "__main__":
  main()

# ======================================================================================================================
