"""
callPointer — зеркало chillffi/examples/callPointer/main.rs.

`scope.pointer(адрес)` — старт цепочки по известному адресу, без `dlopen`:
адрес вернулся из предыдущего вызова C (signal() отдаёт прежний handler)
или прочитан из памяти (таблица диспетчеризации). Адрес живёт в пространстве
клона — из питона напрямую его не позвать (ctypes даёт адреса хоста, они
в клоне невалидны), только через FFI в клоне.

Возвращает тот же `CallBuilder`, что и `lib.имя`; тип результата объявляет
финализатор. Rust-соответствия (дальше — обычная цепочка; в Rust у
callPointer билдера нет — errno там отдельный метод, здесь всё сводится на
одну цепочку):

  scope.pointer(addr).arg(...).result()         ~ Scope::callvPointer (void)
  scope.pointer(addr).arg(...).result("i32")    ~ Scope::callPointer::<i32>
  scope.pointer(addr).arg(...).errno().result() ~ Scope::callPointerErrno
    (errno читается scope.lastErrno())

Запуск: python3 examples/callPointer/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import LibcPath, SignalNumber  # noqa: E402

# Адрес буфера клона для roundtrip-теста — глобал, чтобы тело колбэка
# захватило его снимком при регистрации (как Rust-замыкание захватывает addr).
HandlerAddress: int = 0

# ======================================================================================================================
# Вызов по адресу, который вернул `signal()`.

# Rust: `println!` внутри замыкания — печать из клона доходит до хоста (stderr).
def signalHandlerBody(signum: int) -> None:
  import sys
  sys.stderr.write("[handler] called directly via scope.pointer(...), signum = %d\n" % signum)

# Rust: `callback!(scope, |signum: i32|)` -> `libc.signal.arg(SignalNumber).arg(handler)`,
# затем `libc.signal.arg(SignalNumber).arg(Pointer(0)).result(Pointer)` возвращает адрес
# прежнего обработчика, и `scope.pointer(old).arg(SignalNumber).result()` зовёт
# его напрямую.
def callPointerOnSignalReturnedHandler() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)

    handler: chillffi.Callback = scope.callback("void(i32)", signalHandlerBody)

    # Ставим обработчик. Сигнал никогда не подаётся — signal() только
    # сохраняет и возвращает указатели, доставка тут не важна.
    libc.signal.arg(chillffi.I32(SignalNumber)).arg(handler).result()

    # Снимаем обработчик (SIG_DFL) и забираем то, что signal() отдаёт как
    # «прежний» — ровно адрес, который установили строкой выше. Адрес —
    # в пространстве клона, наружу его не достать, только позвать через FFI.
    old: int = libc.signal.arg(chillffi.I32(SignalNumber)) \
      .arg(chillffi.Pointer(0)).result(chillffi.Pointer)

    # Зовём адрес напрямую, минуя signal(): пустой финализатор = void.
    scope.pointer(old).arg(chillffi.I32(SignalNumber)).result()

  print("ok: the pointer signal() returned was a real, callable callback")

# ======================================================================================================================
# Тот же адрес + запись в память клона — зеркало unit-теста
# chillffi/src/ffi/scope.rs::callPointerAndCallvPointerRoundtrip.

# Rust: `unsafe{ *(addr as *mut i32) = signum; }` — deref в клоне.
def roundtripHandler(signum: int) -> None:
  import ctypes
  ctypes.c_int.from_address(HandlerAddress).value = signum

# Адрес, который вернул signal(), — настоящий code pointer колбэка: запись
# через него видна в буфере клона; захват errno (`.errno().result()`)
# не ломает обычный вызов.
def callPointerAndCallvPointerRoundtrip() -> None:
  global HandlerAddress

  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)
    mem: chillffi.Memory = scope.alloc(4)
    HandlerAddress = mem.address()

    handler: chillffi.Callback = scope.callback("void(i32)", roundtripHandler)

    libc.signal.arg(chillffi.I32(SignalNumber)).arg(handler).result()
    old: int = libc.signal.arg(chillffi.I32(SignalNumber)) \
      .arg(chillffi.Pointer(0)).result(chillffi.Pointer)

    # void-вариант (callvPointer): прямой вызов по адресу — буфер получил 11.
    scope.pointer(old).arg(chillffi.I32(11)).result()
    first: int = int.from_bytes(mem.read(), sys.byteorder)
    assert first == 11, f"first read != 11 ({first})"

    # errno-вариант (callPointerErrno): буфер получил 22.
    scope.pointer(old).arg(chillffi.I32(22)).errno().result()
    errno: int | None = scope.lastErrno()
    second: int = int.from_bytes(mem.read(), sys.byteorder)
    assert second == 22, f"second read != 22 ({second})"

  print("ok: scope.pointer(...).result() wrote 11, .errno().result() wrote 22 (errno =", errno, ")")

# ======================================================================================================================

def main() -> None:
  callPointerOnSignalReturnedHandler()
  callPointerAndCallvPointerRoundtrip()

if __name__ == "__main__":
  main()

# ======================================================================================================================
