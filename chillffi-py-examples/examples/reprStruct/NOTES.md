# reprStruct — реализовано

Зеркало `chillffi/examples/reprStruct/main.rs`.

В Rust типизированный доступ к буферу — это `AllocatedMemory::readStruct::<T>()`
/ `writeStruct::<T>(&T)`, где `T: #[repr(C)] + Pod + Zeroable` (bytemuck):
раскладку гарантирует компилятор, `pod_read_unaligned` переинтерпретирует
байты, размер проверяется против `size_of::<T>()`
(`src/ffi/allocatedMemory.rs:100,137`).

## Как теперь в chillffi-py

| Rust | Python |
|---|---|
| `struct Clock { i64 secs; i64 nanos }` + `#[repr(C)]` | `class Clock(ctypes.Structure): _fields_ = [("secs", ctypes.c_int64), ("nanos", ctypes.c_long)]` |
| `mem.readStruct::<Clock>()` | `mem.readStruct(Clock)` |
| `mem.writeStruct(&original)` | `mem.writeStruct(original)` |
| `size_of::<Clock>()` для `scope.alloc` | `ctypes.sizeof(Clock)` |
| `pod_read_unaligned::<T>(&bytes)` | `cls.from_buffer_copy(bytes)` |

* Роль `#[repr(C)]`-типа играет подкласс `ctypes.Structure`: раскладку считает
  ctypes по тем же C-правилам (нативное выравнивание), `_fields_` — та же
  декларация, что struct-литерал в Rust. Условные формы платформы — как
  `#[cfg(unix)]`/`#[cfg(windows)]` в Rust: `_fields_` фиксируется при
  ОПРЕДЕЛЕНИИ класса, поэтому `reprStruct/main.py` объявляет два варианта
  `Clock` под `if`.
* Читаются ТОЛЬКО байты, которые нужны типу (`ctypes.sizeof(cls)`, как
  `size_of::<T>()` в Rust — блок может быть больше), размер блока проверяется
  с тем же текстом ошибки ядра (`readStruct: buffer is N bytes, but T is M bytes`).
* Отличие от Rust: у bytemuck-`Pod` нет питоновского эквивалента — padding
  компилятором не ловится; ctypes раскладывает по правилам C, как и
  `#[repr(C)]`, поэтому на практике раскладки совпадают, но гарантии
  compile-time здесь нет (проверяется roundtrip-ом в примере).
* Динамические формы (без питоновского класса) — `scope.allocStruct` /
  `readDynamicStruct` / `writeDynamicStruct` — `dynamicStruct/NOTES.md`;
  структуры по значению — `chillffi.StructValue` — `byValueStruct/NOTES.md`.
