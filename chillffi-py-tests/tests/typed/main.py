"""
typed — явные типы аргументов и результата: зеркало `Type` из `chillffi` + `ctypes`.

Голые `int`/`float` в `.arg()` конвертируются по значению (i64/u64/f64) — как в RTS
`tokenToFfiArg` у чисел без явного типа. Явная ширина приходит из своих pyclass-типов
(`chillffi.I32(-7)` — это `arg::<i32>()` в Rust) и из ctypes-инстансов
(`ctypes.c_int32(-7)` — то же самое, тип берётся из `_type_`). Классы работают
и в финализаторе: `.result(chillffi.I32)` = `.result(ctypes.c_int32)` = `.result("i32")`.
Тип результата всегда объявляется финализатором — из значений аргументов он
не выводится (короткой записи нет, билдер нельзя вызвать как функцию).

Механика — та же, что в RTS `bridge.rs`: `FfiArgValue` + typed `pushArg` +
`callWithResult`, только источник явности другой — класс значения в Python
вместо `TokenType` в RTS.

C-помощник собирает `tests/suite.rs` -> `tests/typed/libtyped.so`

Запуск: cargo test
"""
# ======================================================================================================================

import ctypes
import os
import sys

TypedDir: str = os.path.dirname(os.path.abspath(__file__))
# `platformData` общий с примерами и лежит в крейте примеров — копии здесь нет.
sys.path.insert(0, os.path.join(TypedDir, "..", "..", "..", "chillffi-py-examples", "examples", "platform"))

import chillffi  # noqa: E402
from platformData import platformExt  # noqa: E402

# ======================================================================================================================
# Ширина аргумента: голый int уходит i64, а C ждёт int32 — `chillffi.I32` объявляет ширину.
# neg32: int32-аргумент и int32-результат; roll8: uint8.

def typedWidths() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(TypedDir)
    libtyped: chillffi.Library = scope.load(platformExt("libtyped"))

    # Короткой записи нет: тип результата не угадывается — API требует
    # финализатор (отрицательный int32 без явного типа читался бы как
    # 4294967289 — старшие биты rax вне ABI-контракта).
    shortError: str | None = None
    try:
      libtyped.neg32(7)
    except RuntimeError as error:
      shortError = str(error)

    # Явный тип аргумента и результата: `arg::<i32>(7)`, `result::<i32>()`.
    typedResult: int = libtyped.neg32.arg(chillffi.I32(7)).result(chillffi.I32)

    # uint8: 255 + 1 = 0 ровно потому, что аргумент ушёл как u8.
    rolled: int = libtyped.roll8.arg(chillffi.U8(255)).result(chillffi.U8)

  assert typedResult == -7, f"neg32(I32(7)) != -7 ({typedResult})"
  assert shortError and "короткая запись" in shortError, f"neg32(7) не отклонён: {shortError}"
  assert rolled == 0, f"roll8(U8(255)) != 0 ({rolled})"
  print("ok: neg32(7) ->", shortError[:56], "...")
  print("ok: neg32.arg(I32(7)).result(I32) ->", typedResult, " (короткая запись отклонена)")
  print("ok: roll8(U8(255)) ->", rolled)

# ======================================================================================================================
# float-ширина и bool.
# halff: float (f32); twiced: double (f64); truthy: `_Bool`.

def typedFloatBool() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(TypedDir)
    libtyped: chillffi.Library = scope.load(platformExt("libtyped"))

    half: float = libtyped.halff.arg(chillffi.F32(3.0)).result(chillffi.F32)
    # f64-возврат объявлен финализатором: float-аргумент сам тип не выбирает.
    twiced: float = libtyped.twiced.arg(chillffi.F64(2.5)).result("f64")
    truth: int = libtyped.truthy.arg(chillffi.Bool(True)).result(chillffi.I32)

  assert abs(half - 1.5) < 1e-6, f"halff(F32(3.0)) != 1.5 ({half})"
  assert abs(twiced - 5.0) < 1e-12, f"twiced(F64(2.5)) != 5.0 ({twiced})"
  assert truth == 1, f"truthy(Bool(True)) != 1 ({truth})"
  print("ok: halff(F32(3.0)).result(F32) ->", half)
  print("ok: twiced.arg(F64(2.5)).result('f64') ->", twiced)
  print("ok: truthy(Bool(True)) ->", truth)

# ======================================================================================================================
# Строковые: `String` (ptr+len — два C-аргумента из одного питоновского), `CString`, `RawString`.
# `countBytes`/`firstByte`: `(const char*, size_t)` из одного `chillffi.String`.

def typedStrings() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(TypedDir)
    libtyped: chillffi.Library = scope.load(platformExt("libtyped"))

    # Как в RTS: `TokenType::String` -> `FfiArgValue::String` -> (ptr, len).
    # `libtyped.countBytes.arg(String("hello world"))` вместо ручных arg(mem).arg(len).
    length: int = libtyped.countBytes.arg(chillffi.String("hello world")).result(chillffi.Usize)
    first: int = libtyped.firstByte.arg(chillffi.String("abc")).result(chillffi.I8)

    # Явные `CString`/`RawString` — то же, что голые `str`/`bytes`, но тип виден в вызове.
    cLength: int = libtyped.countBytes.arg(chillffi.CString("hey")).arg(chillffi.Usize(2)).result(chillffi.Usize)

  assert length == 11, f"countBytes(String('hello world')) != 11 ({length})"
  assert first == ord("a"), f"firstByte(String('abc')) != 97 ({first})"
  assert cLength == 2, f"countBytes(CString('hey'), Usize(2)) != 2 ({cLength})"
  print("ok: countBytes(String('hello world')) ->", length, "(один питоновский аргумент = ptr+len)")
  print("ok: firstByte(String('abc')) ->", first)
  print("ok: CString/Usize явно ->", cLength)

# ======================================================================================================================
# Pointer: адреса действуют в пространстве клона; NULL — легальный указатель.
# `fill`: запись в память клона по адресу (`Memory` или адрес от C); `isNull`: `NULL`.

def typedPointers() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(TypedDir)
    libtyped: chillffi.Library = scope.load(platformExt("libtyped"))

    mem: chillffi.Memory = scope.alloc(8)

    # Pointer(mem) и Pointer(mem.address()) — одно и то же: адрес клона.
    libtyped.fill.arg(chillffi.Pointer(mem)).arg(chillffi.Usize(8)).arg(chillffi.I8(ord("A"))).result()
    filled: bytes = mem.read()
    libtyped.fill.arg(chillffi.Pointer(mem.address())).arg(chillffi.Usize(4)).arg(chillffi.I8(ord("B"))).result()
    refilled: bytes = mem.read()

    # NULL-указатель (Pointer(0)) — валиден: C честно видит NULL.
    null: int = libtyped.isNull.arg(chillffi.Pointer(0)).result(chillffi.I32)
    nonNull: int = libtyped.isNull.arg(chillffi.Pointer(mem)).result(chillffi.I32)

  assert filled == b"AAAAAAAA", f"fill(Pointer(mem)) != b'AAAAAAAA' ({filled})"
  assert refilled == b"BBBBAAAA", f"fill(Pointer(mem.address())) != b'BBBBAAAA' ({refilled})"
  assert null == 1 and nonNull == 0, f"isNull: NULL={null}, nonNULL={nonNull}"
  print("ok: fill(Pointer(mem), 8, 'A') ->", filled)
  print("ok: fill(Pointer(mem.address()), 4, 'B') ->", refilled)
  print("ok: isNull(Pointer(0)) ->", null, "; isNull(Pointer(mem)) ->", nonNull)

# ======================================================================================================================
# ctypes-интероп: инстансы как аргументы (тип из `_type_`), классы в `.result()`.
# `c_int32` -> `i32`, `c_float` -> `f32`, `c_bool` -> `Bool`, `c_char_p` -> `CString`, NULL -> `Pointer(0)`.

def ctypesInterop() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(TypedDir)
    libtyped: chillffi.Library = scope.load(platformExt("libtyped"))

    # ctypes-инстанс = явный тип: c_int32(7) уходит как i32, а не i64.
    fromCtypes: int = libtyped.neg32.arg(ctypes.c_int32(7)).result(ctypes.c_int32)
    rolled: int = libtyped.roll8.arg(ctypes.c_uint8(255)).result(ctypes.c_uint8)
    half: float = libtyped.halff.arg(ctypes.c_float(3.0)).result(ctypes.c_float)
    truth: int = libtyped.truthy.arg(ctypes.c_bool(True)).result(ctypes.c_int32)

    # Класс-токены в .result(): c_int32 == I32 == "i32".
    viaToken: int = libtyped.neg32.arg(chillffi.I32(7)).result("i32")
    viaClass: int = libtyped.neg32.arg(chillffi.I32(7)).result(chillffi.I32)
    assert viaToken == viaClass == -7, f"result-токены разошлись: {viaToken} / {viaClass}"

    # `c_char_p` -> `CString`; NULL (`c_char_p(None)` / `c_void_p()`) -> `Pointer(0)`.
    cLength: int = libtyped.countBytes.arg(ctypes.c_char_p(b"via ctypes")).arg(chillffi.Usize(10)).result(chillffi.Usize)
    null: int = libtyped.isNull.arg(ctypes.c_void_p()).result(ctypes.c_int32)

  assert fromCtypes == -7, f"neg32(c_int32(7)) != -7 ({fromCtypes})"
  assert rolled == 0, f"roll8(c_uint8(255)) != 0 ({rolled})"
  assert abs(half - 1.5) < 1e-6, f"halff(c_float(3.0)) != 1.5 ({half})"
  assert truth == 1, f"truthy(c_bool(True)) != 1 ({truth})"
  assert cLength == 10, f"countBytes(c_char_p(b'via ctypes')) != 10 ({cLength})"
  assert null == 1, f"isNull(c_void_p()) != 1 ({null})"
  print("ok: neg32(c_int32(7)).result(c_int32) ->", fromCtypes)
  print("ok: roll8(c_uint8(255)) ->", rolled, "; halff(c_float(3.0)) ->", half, "; truthy(c_bool(True)) ->", truth)
  print("ok: c_char_p -> CString ->", cLength, "; c_void_p() -> NULL ->", null)

# ======================================================================================================================
# Границы: свои типы проверяют диапазон; ctypes-адреса хоста отклоняются.
# `I8(300)` — ошибка диапазона; `c_void_p`(адрес хоста) — ошибка изоляции.

def boundaries() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(TypedDir)
    libtyped: chillffi.Library = scope.load(platformExt("libtyped"))

    rangeError: str | None = None
    try:
      libtyped.roll8.arg(chillffi.I8(300)).result()
    except (ValueError, RuntimeError) as error:
      rangeError = str(error)

    hostPointerError: str | None = None
    try:
      libtyped.isNull.arg(ctypes.c_void_p(ctypes.addressof(ctypes.c_int(1)))).result()
    except (ValueError, RuntimeError) as error:
      hostPointerError = str(error)

    byrefError: str | None = None
    try:
      libtyped.isNull.arg(ctypes.byref(ctypes.c_int(1))).result()
    except (ValueError, RuntimeError) as error:
      byrefError = str(error)

    unknownRet: str | None = None
    try:
      libtyped.neg32.arg(chillffi.I32(7)).result(chillffi.CString)
    except (ValueError, RuntimeError) as error:
      unknownRet = str(error)

  assert rangeError and "out of range" in rangeError, f"I8(300) не дал ошибку диапазона: {rangeError}"
  assert hostPointerError and "невалиден в клоне" in hostPointerError, f"c_void_p(хост) не отклонён: {hostPointerError}"
  assert byrefError and "byref" in byrefError, f"byref не отклонён: {byrefError}"
  assert unknownRet and "не читаются как результат" in unknownRet, f".result(CString) не отклонён: {unknownRet}"
  print("ok: I8(300) ->", rangeError)
  print("ok: c_void_p(адрес хоста) ->", hostPointerError)
  print("ok: byref(c_int) ->", byrefError)
  print("ok: .result(chillffi.CString) ->", unknownRet)

# ======================================================================================================================
# Опечатки в типе и загрузка модуля.
# Типы есть, repr-ы осмысленные, `.result()` ловит незнакомые токены.

def moduleSurface() -> None:
  assert chillffi.I32(-7).value == -7 and int(chillffi.U8(255)) == 255
  assert chillffi.F32(2.5).value == 2.5 and chillffi.Bool(True).value is True
  assert repr(chillffi.I32(-7)) == "chillffi.I32(-7)", repr(chillffi.I32(-7))
  assert "chillffi.Pointer(0x" in repr(chillffi.Pointer(16)), repr(chillffi.Pointer(16))
  assert chillffi.CString("hi").value == b"hi" and chillffi.RawString(b"a\x00b").value == b"a\x00b"

  unknownToken: str | None = None
  with chillffi.Scope() as scope:
    scope.addSearchPath(TypedDir)
    libtyped: chillffi.Library = scope.load(platformExt("libtyped"))
    try:
      libtyped.neg32.arg(chillffi.I32(7)).result("i3")
    except (ValueError, RuntimeError) as error:
      unknownToken = str(error)

  assert unknownToken and "unknown result type" in unknownToken, f"result('i3') не отклонён: {unknownToken}"
  print("ok: .value/__int__/__repr__/__float__ у типов работают")
  print("ok: .result('i3') ->", unknownToken)

# ======================================================================================================================

def main() -> None:
  typedWidths()
  typedFloatBool()
  typedStrings()
  typedPointers()
  ctypesInterop()
  boundaries()
  moduleSurface()

if __name__ == "__main__":
  main()

# ======================================================================================================================
