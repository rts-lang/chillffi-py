"""
variadic — зеркало chillffi/examples/variadic/main.rs.

Вариативные FFI-вызовы через `.variadic()` в цепочке.

Запуск: python3 examples/variadic/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import SprintfLibPath  # noqa: E402

# ======================================================================================================================
# `sprintf(char *str, const char *format, ...)` — два фиксированных аргумента, дальше вариативные.
# Rust: `libc.call("sprintf").arg(str).arg(c"Hello %s %d!").variadic()
# .arg(c"world").arg::<i32>(42).result::<i32>()` — все аргументы цепочкой,
# граница фиксированных заморожена `.variadic()`.

def sprintfVariadic() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(SprintfLibPath)
    mem: chillffi.Memory = scope.alloc(64)

    written: int = (
      libc.sprintf
      .arg(mem)                                  # char *str          — фиксированный 1
      .arg(chillffi.CString("Hello %s %d!"))     # const char *fmt    — фиксированный 2 (NUL-терминированный, как c"..." в Rust)
      .variadic()                                # <- всё после этого — вариативное
      .arg(chillffi.CString("world"))            # %s — тоже NUL-указатель, один C-аргумент
      .arg(42)                                   # %d
      .result("i32")
    )

    # mem.read() отдаёт весь блок (мусор после `NUL`), длину даёт `sprintf`.
    data: bytes = mem.read()

  text: str = data[:written].decode("utf-8")
  assert text == "Hello world 42!", f"unexpected sprintf output: {text!r}"
  print("ok: sprintf(...) =", repr(text))

# ======================================================================================================================
# `.variadic()` без фиксированных аргументов отвергается.
# Rust: `FFIError::BadArgument` (`nfixedargs` >= 1). В Python проверка сделана
# на стороне обвязки (`PyCall::variadic` и `prepared()` на финализаторе),
# поэтому это `RuntimeError`, а не `FFIError` из клона.

def variadicWithoutFixedArgsFails() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(SprintfLibPath)
    try:
      libc.sprintf.variadic().arg(chillffi.CString("hello")).result("i32")
      raise SystemExit("вариативный вызов без фиксированных аргументов НЕ упал — плохо")
    except RuntimeError as e:
      assert "variadic" in str(e).lower(), f"unexpected error: {e}"
      print("ok: variadic without fixed args ->", str(e)[:90])

# ======================================================================================================================

def main() -> None:
  sprintfVariadic()
  variadicWithoutFixedArgsFails()

if __name__ == "__main__":
  main()

# ======================================================================================================================
