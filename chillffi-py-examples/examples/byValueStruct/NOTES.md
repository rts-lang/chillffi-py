# byValueStruct — реализовано

Зеркало `chillffi/examples/byValueStruct/main.rs`. В Rust структуры по значению
работают через `StructValue` (список `Arg`) в `CallBuilder::arg` и
`CallBuilder::resultStruct(&[Type])` на выходе; в обвязке — тот же путь 1:1.

## Как в chillffi (Rust)

| Кусок | Где в chillffi |
|---|---|
| `StructValue` как значение-аргумент (`Value::Struct`) | `src/ffi/types/primitive/arg.rs` |
| `Type` (I32/I64/F64/Struct/...) как описание раскладки | `src/ffi/types/mod.rs` |
| `CallBuilder::resultStruct` / `VariadicCallBuilder::resultStruct` | `src/ffi/library.rs:329,416` |
| `StructValue::get` / `getStruct` / `len` / `isEmpty` | `src/ffi/types/primitive/dynamicList.rs` |
| сериализация аргумента в транзитный буфер (structLayout + writeStructAt) | `src/worker.rs` |

## Как теперь в chillffi-py

| Rust | Python |
|---|---|
| `StructValue::new([Arg::from(10i32), Arg::from(32i32)])` | `chillffi.StructValue([chillffi.I32(10), chillffi.I32(32)])` |
| `lib.call("pointSum").arg(point).result()` | `lib.pointSum.arg(point).result("i32")` |
| `.resultStruct(&[Type::I32, Type::I32])` | `.resultStruct([chillffi.Type.I32, chillffi.Type.I32])` |
| `out.get(0)?` / `out.getStruct(0)?` | `out.get(0)` / `out.getStruct(0)` |
| вложенное: `Arg::from(StructValue::new(...))` | вложенный `chillffi.StructValue([...])` в списке полей |

* Поля строятся из ЯВНЫХ типов: голый int ушёл бы как i64 и сломал
  ABI-классификацию (`Arg::from(10i32)` в Rust — здесь `chillffi.I32(10)`);
  скалярные аргументы рядом со структурой — тоже явно (`arg(10i32)` →
  `.arg(chillffi.I32(10))`).
* Поле-указатель — `chillffi.Pointer(...)` с адресом клона (`Memory.address()`
  или адрес, вернувшийся от C); worker сериализует его тем же
  `writeStructAt`. Строка-поле — `chillffi.CString(...)`/`bytes` (один
  указатель); голый `str` (ptr+len, ДВА C-аргумента) полем быть не может —
  как в Rust (`Value::String cannot be a field of a by-value struct`).
* Форма возврата в `.resultStruct([...])` обязательна: возвращаемый буфер
  нетипизирован, пока его не прочитают (вложенность —
  `chillffi.Type.Structure([...])`, аналог `Type::structure`).
* Прочитанное — тот же класс `chillffi.StructValue`: `get(i)` возвращает
  Python-значение тем типом, под которым поле прочитал клон (int/float/bool/
  адрес- int/вложенный StructValue); длину даёт `len(out)`; передать
  прочитанное обратно в `.arg()` можно — FFI-поля восстановлены по форме.
* У `Scope::callPointer` в chillffi формы возврата структуры нет —
  `.resultStruct` работает только на символ-целях (`.pointer(...).resultStruct`
  отклоняется с объяснением).
