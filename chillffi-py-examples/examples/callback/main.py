"""
chillffi-py callback — зеркало chillffi/examples/callback/main.rs.

`callback!` в Rust переправляет closure в клон бит-копией состояния. Питоновское
тело бит-копией переправить нельзя (`PyObject` живёт в куче хоста), поэтому оно
едет маршаленным кодом + pickled-окружением и исполняется тем же
интерпретатором, что поднял зиготу: клон — это `python -I -c
"...ctypes.CDLL(этот .so).chillffiRunZygote(...)"`, ctypes отпустил GIL на
время `chillffiRunZygote`, и трэмполин возвращает его PyGILState_Ensure-ом.

Всё остальное — механика `chillffi`: `scope.callback(...)` строит Sendable
со статическим декодером этого `.so` (у хоста и клона один файл —
`moduleBase() + offset` из callback/addressing.rs), клон декодирует его при
RegisterCallback и оборачивает в libffi-closure. C-код (qsort) зовёт
code pointer, тело исполняется в клоне — там же, где данные, поэтому
указатели в аргументах валидны и разыменовываются честно (ctypes).

Семантика захватов — питоновский аналог «captures строго Copy»:
  * default-args (pickle) и picklable глобальные имена уровня модуля
    (снимок co_names ∩ __globals__ при регистрации);
  * free-переменные (замыкания с ячейками) — не поддержаны;
  * снимок фиксируется при регистрации: изменение глобала на хосте после
    регистрации тело не видит (Copy-семантика, как у бит-копии в Rust);
  * тело не должно звать `chillffi` — оно выполняется внутри C-вызова;
  * исключение Python → traceback в stderr клона → RuntimeError на хосте
    (зеркально панике closure, CallbackPanicked).

Запуск: python3 examples/callback/main.py  (рядом с собранным chillffi.so)
"""
# ======================================================================================================================

import struct

import chillffi

Data: bytes = struct.pack("5i", 3, 1, 4, 1, 5)

# ======================================================================================================================
# Компаратор без захватов — зеркало `noCaptures()` из `chillffi`.

def noCaptures() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load("libc.so.6")
    buf: chillffi.Memory = scope.alloc(20)         # AllocatedMemory: данные в клоне

    buf.write(Data)

    def compar(a: int, b: int) -> int:
      import ctypes
      av: int = ctypes.cast(a, ctypes.POINTER(ctypes.c_int)).contents.value
      bv: int = ctypes.cast(b, ctypes.POINTER(ctypes.c_int)).contents.value
      return (av > bv) - (av < bv)

    cmp: chillffi.Callback = scope.callback("i32(pointer,pointer)", compar)

    libc.qsort.arg(buf).arg(5).arg(4).arg(cmp).result()

    sortedData: tuple[int, ...] = struct.unpack("5i", buf.read())

  assert sortedData == (1, 1, 3, 4, 5)
  print("ok: no-capture comparator ->", sortedData)

# ======================================================================================================================

Bias: int = 3           # сдвигает каждое значение перед сравнением
Descending: bool = True # переворачивает порядок

def flip() -> None:
  global Descending
  Descending = False

# Захваты глобальных имён — зеркало `multipleCapturedVariables()`
# из `chillffi` (там `closure` захватывает `i32` и `bool`; здесь —
# `picklable` глобальные имена).

def multipleCapturedVariables() -> None:
  global Descending
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load("libc.so.6")
    buf: chillffi.Memory = scope.alloc(20)

    buf.write(Data)

    def compar(a: int, b: int) -> int:
      import ctypes
      av: int = ctypes.cast(a, ctypes.POINTER(ctypes.c_int)).contents.value
      bv: int = ctypes.cast(b, ctypes.POINTER(ctypes.c_int)).contents.value
      av += Bias
      bv += Bias
      if Descending:
        return (bv > av) - (bv < av)
      return (av > bv) - (av < bv)

    registeredAt: bool = Descending  # состояние окружения на момент регистрации
    cmp: chillffi.Callback = scope.callback("i32(pointer,pointer)", compar)

    libc.qsort.arg(buf).arg(5).arg(4).arg(cmp).result()
    sortedData: tuple[int, ...] = struct.unpack("5i", buf.read())

    # Copy-семантика: флип на хосте после регистрации тело не видит —
    # повторный прогон с тем же хэндлом сортирует по старому снимку.
    flip()
    buf.write(Data)
    libc.qsort.arg(buf).arg(5).arg(4).arg(cmp).result()
    frozen: tuple[int, ...] = struct.unpack("5i", buf.read())

    # Перерегистрация снимает окружение заново.
    buf.write(Data)
    cmp2: chillffi.Callback = scope.callback("i32(pointer,pointer)", compar)
    libc.qsort.arg(buf).arg(5).arg(4).arg(cmp2).result()
    refreshed: tuple[int, ...] = struct.unpack("5i", buf.read())

  assert sortedData == (5, 4, 3, 1, 1), sortedData
  assert frozen == (5, 4, 3, 1, 1), frozen
  assert refreshed == (1, 1, 3, 4, 5), refreshed
  print(f"ok: multi-capture comparator (bias={Bias}, descending={registeredAt}) ->", sortedData)
  print("ok: same handle after host flip ->", frozen, "(снимок Copy)")
  print("ok: re-registered after flip ->", refreshed)

# ======================================================================================================================

# Исключение в теле не роняет клон: traceback в stderr клона, `RuntimeError`
# на хосте, следующий вызов работает (как после паники closure в Rust).

def errors() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load("libc.so.6")
    buf: chillffi.Memory = scope.alloc(20)

    def boom(a: int, b: int) -> int:
      raise ValueError("boom from the clone")

    bad: chillffi.Callback = scope.callback("i32(pointer,pointer)", boom)
    buf.write(Data)
    try:
      libc.qsort.arg(buf).arg(5).arg(4).arg(bad).result()
      raise SystemExit("исключение колбэка НЕ всплыло — плохо")
    except RuntimeError as e:
      print("ok: exception containment ->", str(e)[:60], "...")

    def compar(a: int, b: int) -> int:
      import ctypes
      av: int = ctypes.cast(a, ctypes.POINTER(ctypes.c_int)).contents.value
      bv: int = ctypes.cast(b, ctypes.POINTER(ctypes.c_int)).contents.value
      return (av > bv) - (av < bv)

    good: chillffi.Callback = scope.callback("i32(pointer,pointer)", compar)
    buf.write(Data)
    libc.qsort.arg(buf).arg(5).arg(4).arg(good).result()
    after: tuple[int, ...] = struct.unpack("5i", buf.read())

  assert after == (1, 1, 3, 4, 5)
  print("ok: клон жив после паники колбэка ->", after)

# ======================================================================================================================

def main() -> None:
  noCaptures()
  multipleCapturedVariables()
  errors()

if __name__ == "__main__":
  main()

# ======================================================================================================================
