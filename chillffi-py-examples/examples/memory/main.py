"""
memory — зеркало chillffi/examples/memory/main.rs.

Четыре границы владения при переносе данных через изоляцию FFI.

1. **Данные питона** — живут на стороне хоста, копируются в клон на время одного
   вызова. C не должен хранить указатель после возврата вызова.
2. **Аллокация chillffi** (`scope.alloc` -> `Memory`) — память в куче клона,
   принадлежит нам, освобождается сама (Drop объекта / закрытие скоупа).
3. **Аллокация C, освобождаем мы** (`strdup` + `scope.readMemory`/`scope.free`) —
   C аллоцировал, мы отпускаем тем же аллокатором (`Scope::free` под ним).
4. **C владеет всем жизненным циклом** — C и аллоцирует, и освобождает внутри
   своего кода. Указатель наружу не уходит.

Аллокатор C намеренно не обёрнут в RAII: только C знает истинную форму
аллокации. chillffi не притворяется владельцем того, что не создавал.

C-помощник собирает `run.rs` (запуск примера) -> `examples/memory/libowned.so`

Запуск: python3 examples/memory/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import (  # noqa: E402
  CloseSymbolName,
  LibcPath,
  PipeSymbolName,
  ReadSymbolName,
  StrdupSymbolName,
  WriteSymbolName,
  platformExt,
)

MemoryDirectory: str = os.path.join(TestDir, "memory")

# ======================================================================================================================
# 1. Данные хоста (в Python — тоже).
# Безопасный шаблон: отдавать C данные, которые она только читает во время
# вызова (`strlen`, `atoi`, ...). Опасный: отдать указатель, который C
# сохраняет на будущее (глобалы, таблицы колбэков).

def rustOwnedInput() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)
    # NUL-терминированный параметр `strlen` — явный `chillffi.CString`
    # (как `c"..."` в Rust; голый `str` был бы `String` = ptr+len, два C-аргумента).
    length: int = libc.strlen.arg(chillffi.CString("hello from rust")).result("usize")

  assert length == 15, f"strlen != 15 ({length})"
  print("ok: rust-owned input   — IPC-копия живёт только на время вызова, len =", length)

# ======================================================================================================================
# 2. Аллокация chillffi (`scope.alloc`).
# `scope.alloc` просит клон сделать `malloc` у себя на куче и отдаёт адрес.
# Нужен, когда нужен буфер, в который C пишет (out-параметры, структуры),
# или который мы передаём в несколько вызовов внутри одного скоупа.
# Владеем мы, C только берёт указатель на время вызова.

def chillffiAllocation() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)

    # int pipefd[2]; — out-параметр, который заполняет pipe().
    # Имена символов платформозависимы (Windows: `_pipe`) — берём из platformData.
    pipe = getattr(libc, PipeSymbolName)
    fdsMem: chillffi.Memory = scope.alloc(8)
    result: int = pipe.arg(fdsMem).result("i32")
    assert result == 0, f"pipe() failed ({result})"

    fdsBytes: bytes = fdsMem.read()
    readFd: int = int.from_bytes(fdsBytes[0:4], sys.byteorder)
    writeFd: int = int.from_bytes(fdsBytes[4:8], sys.byteorder)

    # Второй буфер, который мы тоже владеем — C пишет в него через read().
    bufMem: chillffi.Memory = scope.alloc(2)

    getattr(libc, WriteSymbolName).arg(writeFd).arg(b"hi").arg(2).result()
    getattr(libc, ReadSymbolName).arg(readFd).arg(bufMem).arg(2).result()

    readBytes: bytes = bufMem.read()

    getattr(libc, CloseSymbolName).arg(readFd).result()
    getattr(libc, CloseSymbolName).arg(writeFd).result()

    # `fdsMem` и `bufMem` освобождаются сами при выходе из `with` (Drop объекта).
    # Вручную можно было бы освободить через `scope.free` — как чужие аллокации ниже.
  assert readBytes == b"hi", f"unexpected read: {readBytes!r}"
  print("ok: chillffi allocation — Memory RAII free, pipe ->", readBytes)

# ======================================================================================================================
# 3. Аллокация C, освобождаем мы (`strdup` + `Scope::free`).
# `Memory` тут нет и автоматического освобождения не будет — владение
# остаётся за аллокатором C, пока блок не отпущен явно.
# Rust: `Scope::readMemory` / `Scope::writeMemory` / `Scope::free`;
# Python: `scope.readMemory(addr, n)` / `scope.writeMemory(addr, bytes)` /
# `scope.free(addr)` — тот же набор по произвольному адресу клона
# (`Memory.read`/`write` работает только по своему блоку). Адрес —
# из пространства клона (то, что вернул `strdup`), передаётся как int.

def cAllocationWeFree() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)

    # char *strdup(const char *s) — C аллоцирует, мы получаем только адрес.
    strdup = getattr(libc, StrdupSymbolName)
    ptr: int = strdup.arg(chillffi.CString("owned by C")).result("usize")
    assert ptr != 0, "strdup returned null"

    # Читаем байты, не забирая владение аллокацией (Scope::readMemory в Rust).
    bytesOut: bytes = scope.readMemory(ptr, 10)
    assert bytesOut == b"owned by C", f"unexpected bytes: {bytesOut!r}"

    # Опциональная мутация через тот же канал (Scope::writeMemory в Rust).
    scope.writeMemory(ptr, b"OWNED BY C")
    after: bytes = scope.readMemory(ptr, 10)
    assert after == b"OWNED BY C", f"unexpected bytes: {after!r}"

    # Ручное освобождение — мы не аллокатор, Drop нам не поможет
    # (Scope::free в Rust; тот же аллокатор, что и у chillffi).
    scope.free(ptr)

  print("ok: C alloc, we free   — strdup + readMemory/writeMemory/free, bytes =", bytesOut)

# ======================================================================================================================
# 4. C владеет всем жизненным циклом (`owned.c`).
# `int processOwned(void)` — malloc -> работа -> free -> вернуть сумму;
# `int measureOwnedCopy(char*)` — malloc копии -> strlen -> free -> вернуть длину.
# В отличие от examples/dynamicStruct, вызов `freeData` здесь не нужен:
# уничтожение целиком внутри вызова C.

def cOwnsFullLifecycle() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(MemoryDirectory)
    lib: chillffi.Library = scope.load(platformExt("libowned"))

    # `processOwned`: буфер живёт только внутри C на время вызова.
    sumValue: int = lib.processOwned.result("i32")

    # `measureOwnedCopy`: внутренняя копия, освобождена до возврата.
    length: int = lib.measureOwnedCopy.arg(chillffi.CString("hello")).result("i32")

  assert sumValue == 60, f"processOwned != 60 ({sumValue})"
  assert length == 5, f"measureOwnedCopy != 5 ({length})"
  print(f"ok: C owns lifecycle   — processOwned={sumValue}, measureOwnedCopy={length} (free не из Python)")

# ======================================================================================================================

def main() -> None:
  rustOwnedInput()
  chillffiAllocation()
  cAllocationWeFree()
  cOwnsFullLifecycle()

if __name__ == "__main__":
  main()

# ======================================================================================================================
