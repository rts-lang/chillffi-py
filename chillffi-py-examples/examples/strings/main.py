"""
strings — зеркало chillffi/examples/strings/main.rs.

Три вида строковых аргументов: String (ptr+len), CString (один указатель),
RawString (указатель на байты).

Запуск: python3 examples/strings/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import LibcPath  # noqa: E402

# ======================================================================================================================
# `String`/`&str` → указатель + длина.
# Rust: `libc.call("strnlen").arg("hello world").result()` — `&str` распадался
# на два C-аргумента (ptr + len), что и есть сигнатура `strnlen(s, len)`.
# В Python голый `str` маппится в `String` — тот же ptr+len: один питоновский
# аргумент, клон разворачивает его в два C-аргумента; обёрток не нужно.

def string() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)
    result: int = libc.strnlen.arg("hello world").result("usize")

  assert result == 11, f"strnlen != 11 ({result})"
  print('ok: strnlen("hello world") =', result, " (ptr + len: один аргумент = 2 C-аргумента, как &str в Rust)")

# ======================================================================================================================
# `CString`/`&CStr` → один `\0`-терминированный указатель.
# Rust: `libc.call("strlen").arg(c"chillffi").result()` — литерал `c"..."`
# это `&CStr` → `Value::CString`. В Python явный аналог литерала —
# `chillffi.CString(...)`; голый `str` — это `String` (ptr+len), у `strlen`
# из-за него были бы два лишних C-аргумента.

def cString() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)
    result: int = libc.strlen.arg(chillffi.CString("chillffi")).result("usize")

  assert result == 8, f"strlen != 8 ({result})"
  print('ok: strlen(chillffi.CString("chillffi")) =', result, " (1 C-аргумент: указатель, как c\"...\" в Rust)")

# ======================================================================================================================
# `Vec<u8>`/`&[u8]` → сырой указатель.
# Rust: `libc.call("atoi").arg(b"12345\\0").result()` — `Vec<u8>` это один
# C-аргумент (указатель на байты), без авто-NUL и без длины.
# В Python `bytes` → `RawString` — ровно тот же смысл.

def rawString() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)
    result: int = libc.atoi.arg(b"12345\0").result("i32")

  assert result == 12345, f"atoi != 12345 ({result})"
  print('ok: atoi(b"12345\\0") =', result, " (1 C-аргумент: указатель, без авто-длины/NUL)")

# ======================================================================================================================

def main() -> None:
  string()
  cString()
  rawString()

if __name__ == "__main__":
  main()

# ======================================================================================================================
