"""
byValueStruct — зеркало chillffi/examples/byValueStruct/main.rs.

Структуры по значению: передача и возврат C-структур без out-параметра.
Поля строятся из явных типов (`chillffi.I32(10)` — как `Arg::from(10i32)`
в Rust: голый int ушёл бы как i64 и сломал ABI-классификацию), раскладку
транзитного буфера делает клон (structLayout + libffi ABI). Возврат —
`.resultStruct([...])`: форма возвращаемого буфера объявлена вызывающим,
чтение — `out.get(i)` / `out.getStruct(0)` для вложенного.

Запуск: python3 examples/byValueStruct/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import platformExt  # noqa: E402

ByValueDir: str = os.path.join(TestDir, "byValueStruct")

# ======================================================================================================================
# Маленькая структура, передаваемая по значению (обычно в регистрах).
# Rust: `StructValue::new([Arg::from(10i32), Arg::from(32i32)])` -> `pointSum(Point)`
# с `struct Point { int32_t x; int32_t y; }`.

def pointSum() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(ByValueDir)
    lib: chillffi.Library = scope.load(platformExt("libbyvalue"))

    point: chillffi.StructValue = chillffi.StructValue([chillffi.I32(10), chillffi.I32(32)])
    assert len(point) == 2 and not point.isEmpty(), "StructValue построен неверно"

    sumValue: int = lib.pointSum.arg(point).result("i32")

  assert sumValue == 42, f"pointSum != 42 ({sumValue})"
  print("ok: pointSum(by-value) ->", sumValue)

# ======================================================================================================================
# Структура, возвращаемая по значению.
# Rust: `.resultStruct(&[Type::I32, Type::I32])` -> `StructValue`;
# C: `pointTranslate(Point, dx, dy) -> Point`.

def pointTranslate() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(ByValueDir)
    lib: chillffi.Library = scope.load(platformExt("libbyvalue"))

    point: chillffi.StructValue = chillffi.StructValue([chillffi.I32(1), chillffi.I32(2)])

    out: chillffi.StructValue = (
      lib.pointTranslate
      .arg(point)
      .arg(chillffi.I32(10))
      .arg(chillffi.I32(20))
      .resultStruct([chillffi.Type.I32, chillffi.Type.I32])
    )

    x: int = out.get(0)
    y: int = out.get(1)

  assert (x, y) == (11, 22), f"pointTranslate != (11, 22) ({(x, y)})"
  print("ok: pointTranslate(by-value) ->", (x, y))

# ======================================================================================================================
# Структура больше 16 байт.
# Rust: `struct Big { double a, b, c; int32_t tag; }` — уходит через
# память/стек, классификацию ABI делает libffi.

def bigStruct() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(ByValueDir)
    lib: chillffi.Library = scope.load(platformExt("libbyvalue"))

    # struct Big { f64 a; f64 b; f64 c; i32 tag; } — padding после tag
    # заполняет structLayout/libffi, не вызывающий.
    big: chillffi.StructValue = chillffi.StructValue([
      chillffi.F64(1.5), chillffi.F64(2.5), chillffi.F64(3.0), chillffi.I32(7),
    ])

    sumValue: float = lib.bigSum.arg(big).result("f64")

    big2: chillffi.StructValue = chillffi.StructValue([
      chillffi.F64(1.0), chillffi.F64(2.0), chillffi.F64(3.0), chillffi.I32(4),
    ])

    out: chillffi.StructValue = (
      lib.bigScale
      .arg(big2)
      .arg(chillffi.F64(2.0))
      .resultStruct([chillffi.Type.F64, chillffi.Type.F64, chillffi.Type.F64, chillffi.Type.I32])
    )

    a: float = out.get(0)
    tag: int = out.get(3)

  assert abs(sumValue - 14.0) < 1e-9, f"bigSum = {sumValue}"
  print("ok: bigSum(by-value >16B) ->", sumValue)

  assert abs(a - 2.0) < 1e-9, f"bigScale.a = {a}"
  assert tag == 4, f"bigScale.tag = {tag}"
  print("ok: bigScale(by-value return) -> a =", a, "tag =", tag)

# ======================================================================================================================
# Вложенная структура.
# Rust: `Type::structure([Type::I32, Type::I32])` — вложенная структура:
# в аргументе — вложенный `chillffi.StructValue`, в форме возврата —
# `chillffi.Type.Structure([...])`, чтение — `out.getStruct(0)`.

def nestedStruct() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(ByValueDir)
    lib: chillffi.Library = scope.load(platformExt("libbyvalue"))

    # Nested { Point { i32, i32 }, f32 } — вложенный StructValue = одно поле.
    nested: chillffi.StructValue = chillffi.StructValue([
      chillffi.StructValue([chillffi.I32(3), chillffi.I32(4)]),
      chillffi.F32(2.5),
    ])

    scale: float = lib.nestedScale.arg(nested).result("f32")

    nested2: chillffi.StructValue = chillffi.StructValue([
      chillffi.StructValue([chillffi.I32(5), chillffi.I32(6)]),
      chillffi.F32(1.5),
    ])

    # Форма возврата зеркалит C-определение, включая вложенный Point.
    out: chillffi.StructValue = (
      lib.nestedDouble
      .arg(nested2)
      .resultStruct([
        chillffi.Type.Structure([chillffi.Type.I32, chillffi.Type.I32]),
        chillffi.Type.F32,
      ])
    )

    # Поле 0 — сама структура: достаём getStruct, затем скаляры.
    origin: chillffi.StructValue = out.getStruct(0)
    ox: int = origin.get(0)
    oy: int = origin.get(1)
    sc: float = out.get(1)

  assert abs(scale - 17.5) < 1e-5, f"nestedScale = {scale}"
  print("ok: nestedScale(by-value nested) ->", scale)

  assert (ox, oy) == (10, 12), f"origin = {(ox, oy)}"
  assert abs(sc - 3.0) < 1e-5, f"scale = {sc}"
  print("ok: nestedDouble(by-value nested return) -> origin =", (ox, oy), "scale =", sc)

# ======================================================================================================================

def main() -> None:
  pointSum()
  pointTranslate()
  bigStruct()
  nestedStruct()

if __name__ == "__main__":
  main()

# ======================================================================================================================
