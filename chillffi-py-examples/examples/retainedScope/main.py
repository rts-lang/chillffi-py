"""
retainedScope — зеркало chillffi/examples/retainedScope/main.rs.

`FFIScope::enter` — то же самое, что `ffi!`, но жизненным циклом скоупа
управляешь сам. В Python аналог — блок `with chillffi.Scope() as scope:`:
внутри блока живёт один клон (Scope Retention), снаружи — следующий.

Запуск: python3 examples/retainedScope/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import (  # noqa: E402
  EtcHostnameCString,
  EtcHostnameString,
  LibcPath,
  StatSize,
  StatSymbolName,
  StSizeOffset,
)

# ======================================================================================================================
# Один скоуп — несколько операций.
# `stat("/etc/hostname")` через libc: out-параметр в буфере клона, размер
# файла читается из него по смещению `st_size`. И `load`, и `alloc`, и вызов
# stat идут в одном клоне — pid клона не меняется от вызова к вызову
# (в Rust это и есть `FFIScope::enter`).

def retainedScopeAcrossMultipleOperations() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)

    clonePid: int = getattr(libc, "getpid").result("i32")

    statMem: chillffi.Memory = scope.alloc(StatSize)
    # Rust: `EtcHostnameCString` — литерал `c"..."` (`&CStr`, один NUL-указатель);
    # в Python та же семантика — явный `chillffi.CString(str)` (голый `str` — `String`, ptr+len).
    result: int = getattr(libc, StatSymbolName).arg(chillffi.CString(EtcHostnameCString)).arg(statMem).result("i32")
    assert result == 0, f"stat() returned {result}"

    bytesOut: bytes = statMem.read()

    # Тот же клон: pid не изменился.
    clonePidAgain: int = getattr(libc, "getpid").result("i32")
    assert clonePidAgain == clonePid, f"scope retention broken: {clonePid} != {clonePidAgain}"

  size: int = int.from_bytes(bytesOut[StSizeOffset:StSizeOffset + 8], sys.byteorder)
  expected: int = os.stat(EtcHostnameString).st_size

  assert size == expected, f"file size {size} != {expected}"
  print("file size =", size, "bytes")
  print("ok: stat via libc through retained scope (clone pid =", clonePid, ")")

# ======================================================================================================================

def main() -> None:
  retainedScopeAcrossMultipleOperations()


if __name__ == "__main__":
  main()

# ======================================================================================================================
