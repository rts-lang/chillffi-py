"""
isolation — зеркало chillffi/examples/isolation/main.rs.

Изоляция крашей: краш внутри клона не должен ронять главный процесс.

C-помощник собирает `run.rs` (запуск примера) -> `examples/isolation/libcrash.so`

Запуск: python3 examples/isolation/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import LibmPath, platformExt  # noqa: E402

IsolationDir: str = os.path.join(TestDir, "isolation")

# ======================================================================================================================
# `SIGSEGV` внутри клона -> ошибка, главный процесс жив.

def segfaultIsContained() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(IsolationDir)
    lib: chillffi.Library = scope.load(platformExt("libcrash"))
    try:
      lib.triggerSegfault.result()
      raise SystemExit("упавший клон НЕ сообщил об ошибке — плохо")
    except RuntimeError as e:
      assert "ZygoteCommunicationFailed" in str(e), f"unexpected error: {e}"
      print("ok: segfault contained —", str(e)[:100])

# ======================================================================================================================
# То же самое для `abort()` (`SIGABRT`).

def abortIsContained() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(IsolationDir)
    lib: chillffi.Library = scope.load(platformExt("libcrash"))
    try:
      lib.triggerAbort.result()
      raise SystemExit("упавший клон НЕ сообщил об ошибке — плохо")
    except RuntimeError as e:
      assert "ZygoteCommunicationFailed" in str(e), f"unexpected error: {e}"
      print("ok: abort() contained —", str(e)[:100])

# ======================================================================================================================
# После двух упавших клонов обычный блок `with` всё ещё работает:
# новый `with`-блок = новый клон, упавший клон из прошлого блока не тянет
# за собой остальное (в Rust это отдельный `ffi!`-блок).

def runtimeSurvivesAfterCrash() -> None:
  with chillffi.Scope() as scope:
    libm: chillffi.Library = scope.load(LibmPath)
    result: float = libm.sqrt.arg(16.0).result("f64")

  assert abs(result - 4.0) < 1e-12, f"sqrt(16.0) != 4.0 ({result})"
  print("ok: runtime alive after crashes, sqrt(16) =", result)

# ======================================================================================================================

def main() -> None:
  segfaultIsContained()
  abortIsContained()
  runtimeSurvivesAfterCrash()

if __name__ == "__main__":
  main()

# ======================================================================================================================
