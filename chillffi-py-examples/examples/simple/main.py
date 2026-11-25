"""
simple — зеркало chillffi/examples/simple/main.rs.

Минимальный блок: одна библиотека, один вызов; второй блок — отдельный клон.

Запуск: python3 examples/simple/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import LibcPath, LibmPath  # noqa: E402

# ======================================================================================================================
# Минимальный `ffi!` блок: одна библиотека, один вызов.
# `sqrt` из `libm`: тип результата объявляет финализатор — как
# `.result::<f64>()` в Rust-примере: из значений аргументов тип возврата
# не выводится, короткой записи нет.

def sqrt() -> None:
  with chillffi.Scope() as scope:
    libm: chillffi.Library = scope.load(LibmPath)
    result: float = libm.sqrt.arg(4.0).result("f64")

  assert abs(result - 2.0) < 1e-12, f"sqrt(4.0) != 2.0 ({result})"
  print("ok: sqrt(4.0) =", result)

# ======================================================================================================================
# `abs(-5)` — отдельный `with`-блок, то есть отдельный клон.
# Отличие от Rust-примера: там `abs` грузится из `libm` (там `libm` экспортирует
# его — macOS `libSystem`). На `glibc` `abs` лежит в `libc.so.6`, а `libm.so.6` его
# не экспортирует, поэтому здесь грузим libc. Тип результата объявляет
# финализатор (`.result("i32")`), как в Rust-примере `result::<i32>()`:
# без явного i32 отрицательный -5 читался бы как 4294967291 — старшие биты
# rax вне ABI-контракта.

def multipleCallsSameScope() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)
    result: int = libc.abs.arg(-5).result("i32")

  assert result == 5, f"abs(-5) != 5 ({result})"
  print("ok: abs(-5) =", result)

# ======================================================================================================================

def main() -> None:
  sqrt()
  multipleCallsSameScope()

if __name__ == "__main__":
  main()

# ======================================================================================================================
