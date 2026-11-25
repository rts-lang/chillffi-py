## chillffi-py

**A simple isolated dynamic FFI framework for Python**

<!-- todo Здесь всякие бейджи -->

`chillffi` allows dynamically loading **C ABI-compatible libraries**
and calling their functions at runtime, **isolating each FFI call in a separate process**.
If third-party native code crashes or corrupts memory,
the failure is contained within the isolated process,
keeping your main Python application running.

Эта реализация построена из Rust версии [`chillffi`](https://github.com/rts-lang/chillffi) через PyO3.
Почему так, а не через C версию: C версия накладывает ручной контроль,
поэтому пришлось писать бы обвязки, работу с памятью и другие штуки,
что не удобно в интеграции и поддержке.

---

### ✨ Features

Все features взяты из [оригинала](https://github.com/rts-lang/chillffi#-features) features в `chillffi`,
однако их статусы могут отличаться из-за особенностей Python.

Запустить пример можно через: `cargo ex <example>`.

По этой причине сделан `workspace`, чтобы отделить Python примеры от `cdynlib` на Rust.

| Feature                                                                         | Status                                                                                                                                        |
|---------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------|
| Native FFI execution                                                            | ✅ Not a VM, not large in size, and does not require large dependencies.                                                                      |
| IPC                                                                             | ✅ `ipc-channel` provides serialization and is implemented for different platforms.                                                           |
| Startup speed                                                                   | ✅ The Zygote model does not retain garbage and uses a fast `fork` for each FFI.                                                              |
| [Multithreading and async](chillffi-py-examples/examples/threads/main.py)       | ✅ GIL отпускается на время IPC (`py.detach`); клон живёт на потоке-владельце скоупа, вызовы с других потоков и из `asyncio.to_thread` сериализуются в тот же клон. |
| [Crash isolation](chillffi-py-examples/examples/isolation/main.py)              | ✅ A separate process for FFI that does not break your Runtime.                                                                               |
| [Scope](chillffi-py-examples/examples/simple/main.py)                           | ✅ Scope for FFI execution areas (`with chillffi.Scope()`);<br/>so that they are short and do not escape.                                     |
| [Retained Scope](chillffi-py-examples/examples/retainedScope/main.py)           | ✅ Temporary retention of the FFI scope for dynamic systems.                                                                                  |
| [Dynamic loading](chillffi-py-examples/examples/library/main.py)                | ✅ `libffi` is simple, stable, cross-platform, and small in size.                                                                             |
| [Path resolver](chillffi-py-examples/examples/paths/main.py)                    | ✅ `chillffi.addGlobalSearchPath(dir)` — глобальный путь поиска (общий на процесс); резолв `scope.load` делает ядро, 1:1: локальный путь скоупа → глобальный → путь как дан.                                              |
| Static FFI                                                                      | ❌ Статического пути из Python нет (`ffi!` — это Rust);<br/>обвязка только динамическая.                                                      |
| [Dynamic FFI](chillffi-py-examples/examples/library/main.py)                    | ✅ On-the-fly function calls without the need to compile static C bindings.                                                                   |
| [Static structures](chillffi-py-examples/examples/reprStruct/main.py)           | ✅ `mem.readStruct(cls)` / `mem.writeStruct(inst)` — раскладка из подкласса `ctypes.Structure` (`_fields_`), как `#[repr(C)]` + Pod в Rust.          |
| [Dynamic structures](chillffi-py-examples/examples/dynamicStruct/main.py)       | ✅ Движок `Type`: `chillffi.Type.*` / `Type.Structure([...])`, `scope.allocStruct`, `scope.readDynamicStruct`/`writeDynamicStruct` — раскладка ABI-математикой libffi на клоне.                            |
| [Pointer-based structures](chillffi-py-examples/examples/dynamicStruct/main.py) | ✅ Поля-указатели — `chillffi.Pointer(...)` (адрес клона) в `writeDynamicStruct` и в полях `StructValue`; чтение массива по адресу из C — `scope.readMemory`.                                             |
| [By-value structures](chillffi-py-examples/examples/byValueStruct/main.py)      | ✅ `chillffi.StructValue([...])` в `.arg()` и `.resultStruct([...])` на билдере (в т.ч. >16 байт и вложенные; чтение — `get`/`getStruct`).                                                              |
| [Allocation handling](chillffi-py-examples/examples/memory/main.py)             | ✅ Allocation of a memory region for FFI.                                                                                                     |
| [Callbacks](chillffi-py-examples/examples/callback/main.py)                     | ✅ Passing Python closures as C functions.                                                                                                    |
| [Signals](chillffi-py-examples/examples/callPointer/main.py)                    | ✅ `scope.pointer(addr)` возвращает тот же `CallBuilder`, что и `lib.имя`: `.arg(x).result("i32")` ~ `callPointer::<i32>`, `.errno().result(...)` ~ `callPointerErrno`, пустой `.result()` ~ `callvPointer` — вызов по адресу из клона, без dlopen.              |
| [Errno Policy](chillffi-py-examples/examples/errno/main.py)                     | ✅ Полный приоритет 1:1 с `resolveReadErrno`: per-call (`.errno()`/`.noErrno()`) > scope (`scope.setReadErrno()`) ><br/>глобальный дефолт (`chillffi.setGlobalReadErrno()`, изначально `false`).                                      |
| [String data types](chillffi-py-examples/examples/strings/main.py)              | ✅ `str` → `String` (ptr+len — ДВА C-аргумента, как `&str` в Rust), `bytes` → `RawString` (как `b"..."`), `chillffi.CString(...)` → один NUL-указатель (как `c"..."`); явные типы `I8..U64`/`F32`/`Pointer` с интеропом ctypes.  |
| [Variadic arguments](chillffi-py-examples/examples/variadic/main.py)            | ✅ Support for variable arguments in function calls.                                                                                          |
| Sandbox (FS protection)                                                         | ⏳ [#45](https://github.com/rts-lang/chillffi/issues/45).                                                                                     |
| Libraries from bytes                                                            | ⏳ [#42](https://github.com/rts-lang/chillffi/issues/42).                                                                                     |
