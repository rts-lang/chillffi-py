"""
dynamicStruct — зеркало chillffi/examples/dynamicStruct/main.rs.

Динамические структуры: раскладка задаётся в рантайме списком `chillffi.Type`,
Rust-типа (и питоновского класса раскладки) не требуется. Размер, смещения и
выравнивание считает клон той же ABI-математикой libffi, что и вызовы:
`scope.allocStruct([...])`, `scope.readDynamicStruct(addr, [...])`,
`scope.writeDynamicStruct(...)`. Поля-указатели — `chillffi.Pointer(...)`
с адресом клона; чтение массива по указателю — `scope.readMemory`.

C-помощники собирает `run.rs` (запуск примера) -> `examples/dynamicStruct/libparameter.so`
и `examples/dynamicStruct/libresult.so`

Запуск: python3 examples/dynamicStruct/main.py
"""
# ======================================================================================================================

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
  platformExt,
)

DynamicDir: str = os.path.join(TestDir, "dynamicStruct")

# ======================================================================================================================
# Чтение out-параметра ОС (структура timespec) по форме, известной в рантайме.
# Rust: `Scope::readDynamicStruct(mem.address(), &[Type::I64, Type::I64])`.
# Форма — список `chillffi.Type`; поля возвращаются тем типом, под которым
# их прочитал клон (`fields.get(0)` -> int). Без `struct.Struct`: раскладку
# и смещения считает движок ядра.

def readDynamicStruct() -> None:
  with chillffi.Scope() as scope:
    clockLib: chillffi.Library = scope.load(TimeLibPath)
    mem: chillffi.Memory = scope.alloc(16)

    secs: int
    nanos: int

    if FileTimeUnixEpoch is None:
      # Unix: clock_gettime(CLOCK_REALTIME, struct timespec { i64, i64 }).
      getattr(clockLib, TimeSymbolName).arg(ClockRealtime).arg(mem).result()
      fields: chillffi.StructValue = scope.readDynamicStruct(
        mem.address(), [chillffi.Type.I64, chillffi.Type.I64]
      )
      secs, nanos = fields.get(0), fields.get(1)
    else:
      # Windows: GetSystemTimeAsFileTime(FILETIME { u64 ticks }).
      getattr(clockLib, TimeSymbolName).arg(mem).result()
      fields: chillffi.StructValue = scope.readDynamicStruct(mem.address(), [chillffi.Type.U64])
      ticks: int = fields.get(0)
      sinceEpoch: int = max(0, ticks - FileTimeUnixEpoch)
      secs = sinceEpoch // 10_000_000
      nanos = (sinceEpoch % 10_000_000) * 100

  print("ok: readDynamicStruct -> realtime =", f"{secs}.{nanos:09}")

# ======================================================================================================================
# Структура как аргумент вызова.
# C: `struct Data { int size; int *values; }; void process(struct Data *data);`
# Rust: `Scope::allocStruct(&dataShape)` + `Scope::writeDynamicStruct(...)`.
# Размер структуры не считается на вызывающем: его отдаёт клон (ABI-математика,
# 4 байта int + 4 байта выравнивания + 8 байт указателя = 16 на x86_64).
# Поле-указатель — `chillffi.Pointer(valuesMem)` с адресом клона.

def structAsCallParameter() -> None:
  dataShape: list[chillffi.Type] = [chillffi.Type.I32, chillffi.Type.Pointer]
  values: tuple[int, ...] = (10, 20, 30)

  with chillffi.Scope() as scope:
    scope.addSearchPath(DynamicDir)
    lib: chillffi.Library = scope.load(platformExt("libparameter"))

    valuesMem: chillffi.Memory = scope.alloc(len(values) * 4)
    valuesMem.write(b"".join(v.to_bytes(4, sys.byteorder, signed=True) for v in values))

    dataMem: chillffi.Memory = scope.allocStruct(dataShape)
    scope.writeDynamicStruct(dataMem.address(), dataShape,
                             [chillffi.I32(len(values)), chillffi.Pointer(valuesMem)])
    assert dataMem.length() == 16, f"allocStruct вернул {dataMem.length()} байт (ожидали 16)"

    lib.process.arg(dataMem).result()

    # process() только читал из `data` — владение аллокацией осталось у нас,
    # блоки освобождаются сами при выходе из with.
    sumValue: int = lib.getSum.result("i32")

  assert sumValue == 60, f"sum != 60 ({sumValue})"
  print("ok: dynamic struct as a call parameter -> sum =", sumValue)

# ======================================================================================================================
# Структура как результат вызова.
# C: `struct Data { int size; int *values; }; struct Data *process(void);`
# Rust: `Scope::readDynamicStruct(dataPtr, &[Type::I32, Type::Pointer])` —
# тот же движок читает структуру по указателю, вернувшемуся из C; массив
# значений дочитывается `Scope::readMemory(valuesPtr, size * 4)`.

def structAsCallResult() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(DynamicDir)
    lib: chillffi.Library = scope.load(platformExt("libresult"))

    dataPtr: int = lib.process.result("pointer")
    assert dataPtr != 0, "process() returned NULL"

    dataShape: list[chillffi.Type] = [chillffi.Type.I32, chillffi.Type.Pointer]
    fields: chillffi.StructValue = scope.readDynamicStruct(dataPtr, dataShape)
    size: int = fields.get(0)
    valuesPtr: int = fields.get(1)

    bytesOut: bytes = scope.readMemory(valuesPtr, size * 4)
    values: tuple[int, ...] = tuple(
      int.from_bytes(bytesOut[i:i + 4], sys.byteorder, signed=True) for i in range(0, len(bytesOut), 4)
    )

    # `process()` аллоцировал и структуру, и массив — освобождаем через
    # собственный destroy-API стороны C (`freeData`), потому что только C
    # знает истинную форму аллокации.
    lib.freeData.arg(dataPtr).result()

  assert values == (10, 20, 30), f"unexpected values: {values}"
  print("ok: dynamic struct as a call result ->", values)

# ======================================================================================================================
# Вложенная форма.
# Rust: `Scope::allocStruct` со вложенным `Type::Struct` — движок раскладки
# сам считает размер с учётом выравнивания (`ffi_get_struct_offsets`).
# Вложенность задаётся `chillffi.Type.Structure([...])` (аналог
# `Type::structure([...])` в Rust). Чтение вложенного поля по публичному API
# ядра ограничено (`get::<T>` — только скаляры); здесь проверяется размер,
# который посчитал клон.

def nestedStructShapeIsSized() -> None:
  shape: list[chillffi.Type] = [
    chillffi.Type.Structure([chillffi.Type.U64, chillffi.Type.U8, chillffi.Type.F64]),
    chillffi.Type.I8,
    chillffi.Type.I64,
  ]

  with chillffi.Scope() as scope:
    mem: chillffi.Memory = scope.allocStruct(shape)
    size: int = mem.length()

  assert size > 0, f"allocStruct вернул размер {size}"
  print(f"ok: nested Type.Structure shape sized to {size} bytes (extraction not yet public)")

# ======================================================================================================================

def main() -> None:
  readDynamicStruct()
  structAsCallParameter()
  structAsCallResult()
  nestedStructShapeIsSized()

if __name__ == "__main__":
  main()

# ======================================================================================================================
