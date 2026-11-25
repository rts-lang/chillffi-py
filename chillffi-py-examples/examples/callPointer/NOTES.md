# callPointer — реализовано

Зеркало `chillffi/examples/callPointer/main.rs`, плюс — зеркало unit-теста
`chillffi/src/ffi/scope.rs::callPointerAndCallvPointerRoundtrip`
(запись в память клона через вызванный по адресу обработчик).

## Как в `chillffi` (Rust)

* `Scope::callPointer<T>(pointer, args)` — `src/ffi/scope.rs:322`;
* `Scope::callvPointer(pointer, args)` — `src/ffi/scope.rs:334`, void-вариант
  (fire-and-forget — зеркало `CallBuilder::void`, НЕ variadic);
* `Scope::callPointerErrno<T>(pointer, args)` — `src/ffi/scope.rs:347`,
  форсирует захват errno для конкретного вызова (у `callPointer` нет билдера,
  куда можно прицепить `.errno()`).

Оба ходят в клон тем же IPC, что и обычный вызов, только адрес приходит
не из реестра библиотек (dlopen), а извне: вернулся из предыдущего вызова
(`signal()` возвращает прежний handler), прочитан из памяти (таблица
диспетчеризации) или это `AllocatedMemory`-адрес.

## Как теперь в chillffi-py

| Rust | Python |
|---|---|
| `Scope::callvPointer(old, vec![Arg::from(11i32)])` | `scope.pointer(old).arg(chillffi.I32(11)).result()` |
| `Scope::callPointer::<i32>(old, args)` | `scope.pointer(old).arg(...).result("i32")` |
| `Scope::callPointerErrno::<()>(old, args)` | `scope.pointer(old).arg(...).errno().result()` |

* Вызов по адресу — СТАРТ ТОЙ ЖЕ ЦЕПОЧКИ (`scope.pointer(адрес)`): дальше
  `.arg()` / `.errno()` / финализатор `.result(...)` — общий `PyCall` с
  символ-целями. Отдельных `callPointer`/`callvPointer`/`callPointerErrno` в
  Python API нет — как и в Rust нет отдельного билдера у pointer-вызовов.
* Адрес — `int` из пространства клона (то, что вернул `signal()`/`calloc`,
  `mem.address()`), либо сам `Memory`/`chillffi.Pointer` (метод снимает
  `.address`). Голый `int` — как `impl Into<usize>` в Rust: валидность —
  на вызывающем, адреса хоста в клоне невалидны (изоляция переживёт промах).
* Аргументы — те же, что у `.arg()`: Infer (int → i64/u64, float → f64,
  str → String ptr+len, bytes → RawString), наши типы (`chillffi.I32(...)`),
  ctypes-инстансы, `Memory`/`Pointer`, зарегистрированный `Callback`,
  `chillffi.StructValue` (по значению — worker сериализует в транзитный буфер).
* Тип результата — финализатором: токены
  `f64|f32|i64|i32|i16|i8|isize|u64|u32|u16|u8|usize|pointer|bool|void`;
  `.result()` без типа — void (зеркало `callvPointer`). `pointer` читает
  возвращённый адрес как число (`signal()` возвращает `void (*)(int)`).
* errno: `.errno()` — прямой аналог `callPointerErrno`, читается
  `scope.lastErrno()`.
* Вариативной формы у `Scope::callPointer` в chillffi нет — `.variadic()` на
  pointer-цели отклоняется; формы возврата структуры тоже нет —
  `.resultStruct` работает только на символ-целях.
