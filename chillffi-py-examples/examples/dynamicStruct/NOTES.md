# dynamicStruct — реализовано

Зеркало `chillffi/examples/dynamicStruct/main.rs`.

В Rust движок динамических структур — это три метода:

| Метод | Что делает | Где в `chillffi` |
|---|---|---|
| `Scope::allocStruct(&[Type])` | выделяет в куче клона блок под структуру по форме (считает размер с выравниванием) | `src/ffi/scope.rs:201` |
| `Scope::readDynamicStruct(ptr, &[Type])` | читает поля структуры по форме | `src/ffi/scope.rs:282` |
| `Scope::writeDynamicStruct(ptr, &[Type], Vec<Arg>)` | пишет поля структуры по форме | `src/ffi/scope.rs:300` |

Формы задаются типом `Type` (`src/ffi/types/mod.rs`): `I8..U64`, `F32`, `F64`,
`Bool`, `Pointer`, `Struct([Type])`. Размер/смещения/выравнивание считает КЛОН
(`structLayout` — `ffi_get_struct_offsets`), не вызывающий.

## Как теперь в chillffi-py

| Rust | Python |
|---|---|
| `scope.allocStruct(&dataShape)` | `scope.allocStruct([chillffi.Type.I32, chillffi.Type.Pointer])` |
| `Scope::readDynamicStruct(addr, &shape)` | `scope.readDynamicStruct(addr, shape)` |
| `Scope::writeDynamicStruct(addr, &shape, vec![Arg::from(...), ...])` | `scope.writeDynamicStruct(addr, shape, [chillffi.I32(n), chillffi.Pointer(mem)])` |
| `Scope::readMemory(valuesPtr, size * 4)` | `scope.readMemory(valuesPtr, size * 4)` |
| `fields.get(0)?` / `fields.getStruct(0)?` | `fields.get(0)` / `fields.getStruct(0)` |

* Форма — список `chillffi.Type` (зеркало `Type`: `chillffi.Type.I64`,
  `chillffi.Type.Pointer`, вложенная — `chillffi.Type.Structure([...])`,
  аналог `Type::structure`).
* Значения в `writeDynamicStruct` — те же аргументы, что в `.arg()`;
  ширина должна совпадать с типом поля (`chillffi.I32(n)` под `Type.I32` —
  голый int ушёл бы как i64 и был бы отвергнут движком с тем же текстом,
  что в Rust).
* Поле-указатель — `chillffi.Pointer(...)` с адресом клона; чтение массива
  по адресу из C — `scope.readMemory` (статический `Scope::readMemory` в Rust;
  `Memory.read`/`write` работает только по СВОЕМУ блоку). Запись по адресу —
  `scope.writeMemory`, освобождение чужой аллокации — `scope.free`.
* Прочитанное — `chillffi.StructValue`: `get(i)` возвращает значение тем
  типом, под которым поле прочитал клон; размер — `len(fields)`.
* Чтение вложенного поля: в ядре публичное извлечение вложенного поля
  (`get::<T>`) пока ограничено скалярами — `main.py` отражает то же
  ограничение (кейс размера вложенной формы), это фича-запрос в chillffi,
  не в обвязку.
