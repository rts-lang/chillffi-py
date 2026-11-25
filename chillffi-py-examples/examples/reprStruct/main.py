"""
reprStruct — зеркало chillffi/examples/reprStruct/main.rs.

Типизированный доступ к буферу: `readStruct` / `writeStruct` в Rust
(`#[repr(C)]` + `Pod`), здесь — те же методы на `Memory`, а роль
`#[repr(C)]`-типа играет подкласс `ctypes.Structure` (`_fields_` — та же
декларация C-раскладки; раскладку считает ctypes по правилам C ABI,
`from_buffer_copy` — аналог `pod_read_unaligned`).

Запуск: python3 examples/reprStruct/main.py
"""
# ======================================================================================================================

import ctypes
import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import (  # noqa: E402
  ClockRealtime,
  FileTimeUnixEpoch,
  TimeLibPath,
  TimeSymbolName,
)

# Разбор `timespec: { time_t tv_sec; long tv_nsec; }` — `#[repr(C)] Clock`
# в Rust: раскладку декларирует ctypes (c_long = 64 бита на unix LP64).
# `_fields_` фиксируется при определении класса — формы задаются условно,
# как два `#[cfg(unix)]`/`#[cfg(windows)]`-варианта struct Clock в Rust.
if FileTimeUnixEpoch is None:
  class Clock(ctypes.Structure):
    _fields_ = [("secs", ctypes.c_int64), ("nanos", ctypes.c_long)]

    def split(self) -> tuple[int, int]:
      return (self.secs, self.nanos)
else:
  # Windows: FILETIME — один u64 тиков по 100нс от 1601-01-01.
  class Clock(ctypes.Structure):
    _fields_ = [("ticks", ctypes.c_uint64)]

    def split(self) -> tuple[int, int]:
      sinceEpoch: int = max(0, self.ticks - FileTimeUnixEpoch)
      return (sinceEpoch // 10_000_000, (sinceEpoch % 10_000_000) * 100)

# Обычный 16-байтный POD, не привязанный к раскладке ОС (Sample в Rust).
class Sample(ctypes.Structure):
  _fields_ = [("a", ctypes.c_int64), ("b", ctypes.c_int64)]

# ======================================================================================================================
# Ручной разбор: слайсы и `from_ne_bytes`.

def manualByteParsing() -> None:
  with chillffi.Scope() as scope:
    clockLib: chillffi.Library = scope.load(TimeLibPath)
    mem: chillffi.Memory = scope.alloc(ctypes.sizeof(Clock))

    if FileTimeUnixEpoch is None:
      # Unix: `clock_gettime(CLOCK_REALTIME, struct timespec *)`.
      getattr(clockLib, TimeSymbolName).arg(ClockRealtime).arg(mem).result()
    else:
      # Windows: `GetSystemTimeAsFileTime(FILETIME*)` — один указатель.
      getattr(clockLib, TimeSymbolName).arg(mem).result()

    bytesOut: bytes = mem.read()
    clock: Clock = Clock.from_buffer_copy(bytesOut)
    secs, nanos = clock.split()

  print(f"manual:  realtime = {secs}.{nanos:09}")

# ======================================================================================================================
# То же самое через `readStruct` — без ручного разбора байт.
# Rust: `mem.readStruct::<Clock>()` — раскладку берёт из `#[repr(C)]` POD.
# Здесь `mem.readStruct(Clock)`: читаются только байты, которые нужны типу
# (`ctypes.sizeof(Clock)`, как `size_of::<T>()` в Rust), размер блока
# проверяется с тем же текстом ошибки.

def readStructRemovesManualParsing() -> None:
  with chillffi.Scope() as scope:
    clockLib: chillffi.Library = scope.load(TimeLibPath)
    mem: chillffi.Memory = scope.alloc(ctypes.sizeof(Clock))

    if FileTimeUnixEpoch is None:
      getattr(clockLib, TimeSymbolName).arg(ClockRealtime).arg(mem).result()
    else:
      getattr(clockLib, TimeSymbolName).arg(mem).result()

    clock: Clock = mem.readStruct(Clock)
    secs, nanos = clock.split()

  print(f"typed:   realtime = {secs}.{nanos:09}")

# ======================================================================================================================
# `writeStruct` -> `readStruct` roundtrip.

def writeThenReadStructRoundtrip() -> None:
  with chillffi.Scope() as scope:
    mem: chillffi.Memory = scope.alloc(ctypes.sizeof(Sample))

    original: Sample = Sample(a=1_700_000_000, b=123_456_789)
    mem.writeStruct(original)

    readBack: Sample = mem.readStruct(Sample)

  assert (original.a, original.b) == (readBack.a, readBack.b), \
    f"roundtrip mismatch: {(original.a, original.b)} != {(readBack.a, readBack.b)}"
  print("ok: writeStruct/readStruct roundtrip ->", (readBack.a, readBack.b))

# ======================================================================================================================
# Недостаточно большой буфер — ошибка с текстом ядра.
# Rust-тест `readStructBufferTooSmall`: буфер меньше `size_of::<T>`
# отказывается читать — проверка на стороне обвязки.

def readStructBufferTooSmall() -> None:
  refused: str | None = None
  with chillffi.Scope() as scope:
    mem: chillffi.Memory = scope.alloc(8)  # Sample требует 16
    try:
      mem.readStruct(Sample)
    except RuntimeError as e:
      refused = str(e)

  assert refused and "readStruct: buffer is" in refused, f"малый буфер не отклонён: {refused}"
  print("ok: readStruct в маленький буфер ->", refused[:80])

# ======================================================================================================================

def main() -> None:
  manualByteParsing()
  readStructRemovesManualParsing()
  writeThenReadStructRoundtrip()
  readStructBufferTooSmall()

if __name__ == "__main__":
  main()

# ======================================================================================================================
