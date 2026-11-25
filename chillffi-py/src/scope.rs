use crate::callback::{PyCallback, buildCallbackSendable, parseCallbackSignature};
use crate::error::toPyErr;
use crate::library::{PyCall, PyLib};
use crate::memory::PyMemory;
use crate::pytypes::TypedPointer;
use crate::structs::{argsFromPy, shapeFromPy, structValueToPython};
use chillffi::ffi::callback::sendable::Sendable;
use chillffi::ffi::errors::FFIError;
use chillffi::ffi::library::Library;
use chillffi::ffi::scope::{FFIScope, Scope as ChillScope};
use chillffi::ffi::types::Type as FfiType;
use chillffi::ffi::types::primitive::Callback as FfiCallback;
use chillffi::ffi::types::primitive::StructValue as FfiStructValue;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
// =================================================================================================
// scope.rs — удерживаемый FFI-скоуп и его питоновская обёртка.
//
// `ScopeSlot` — ленивый слот. `FFIScope` живёт на выделенном потоке-владельце:
// стек зигот `chillffi` thread-local, поэтому клон нельзя ни перенести, ни позвать
// с чужого потока. Любой поток хоста (и asyncio.to_thread) шлёт работу владельцу
// и ждёт ответ. Дроп клона тоже на владельце — `ZygoteGuard` снимает именно свой
// стек. `Scope` — `#[pyclass]`: вход/выход `with`-блока, `addSearchPath`,
// `lastErrno`, `alloc` и `callback`. Зиготу поднимает сам `chillffi`.
// =================================================================================================

/// Работа для потока-владельца. Замыкание исполняется только там,
/// где `FFIScope::enter` положил клон в thread-local стек.
enum WorkerMsg
{
  Run(Box<dyn FnOnce(&FFIScope) + Send>),
  Shutdown
}

/// Состояние питоновского `Scope`, разделяемое со всеми его библиотеками.
/// Сам `FFIScope` не `Send`: он торчит только в потоке-владельце.
pub struct ScopeSlot
{
  /// Канал к потоку-владельцу. `None` до первой операции.
  tx: Mutex<Option<Sender<WorkerMsg>>>,

  /// Поток-владелец. `join` на выходе из блока — клон умирает на своём потоке.
  worker: Mutex<Option<JoinHandle<()>>>,

  /// scope-уровень errno-политики: `Some(v)` — переопределение для всех
  /// вызовов скоупа. Приоритет уровней — `errnoPolicy.rs`.
  readErrno: Mutex<Option<bool>>,

  /// `LastErrno` ядра — thread-local потока-владельца. Сюда копируется
  /// после каждой операции, чтобы `scope.lastErrno()` читался с любого потока.
  lastErrno: Mutex<Option<i32>>,

  /// После выхода из `with` скоуп закрыт: вызовы библиотек — ошибка.
  closed: AtomicBool
}

impl ScopeSlot
{
  // todo desc
  fn new() -> Self
  {
    ScopeSlot
    {
      tx: Mutex::new(None),
      worker: Mutex::new(None),
      readErrno: Mutex::new(None),
      lastErrno: Mutex::new(None),
      closed: AtomicBool::new(false)
    }
  }

  /// Ленивый старт потока-владельца. `FFIScope::enter` — на нём же.
  fn ensureWorker(&self) -> Result<(), FFIError>
  {
    let mut txSlot: MutexGuard<'_, Option<Sender<WorkerMsg>>> =
      self.tx.lock().expect("scope mutex poisoned");
    if self.closed.load(Ordering::SeqCst)
    {
      return Err(FFIError::Other("scope is closed".to_string()));
    }
    if txSlot.is_some()
    {
      return Ok(());
    }

    let (tx, rx): (Sender<WorkerMsg>, Receiver<WorkerMsg>) = mpsc::channel();
    let (readyTx, readyRx) = mpsc::channel::<Result<(), String>>();

    let handle: JoinHandle<()> = thread::Builder::new()
      .name("chillffi-scope".to_string())
      .spawn(move ||
      {
        let ffiScope: FFIScope = match FFIScope::enter()
        {
          Ok(scope) => scope,
          Err(error) =>
          {
            let _ = readyTx.send(Err(error.to_string()));
            return;
          }
        };
        let _ = readyTx.send(Ok(()));
        while let Ok(msg) = rx.recv()
        {
          match msg
          {
            WorkerMsg::Shutdown => break,
            WorkerMsg::Run(job) => job(&ffiScope)
          }
        }
        // Дроп на том же потоке, что и `ZygoteGuard::enter`.
        drop(ffiScope);
      })
      .map_err(|error| FFIError::Other(format!("failed to start scope worker: {error}")))?;

    match readyRx.recv()
    {
      Ok(Ok(())) => {}
      Ok(Err(error)) => return Err(FFIError::Other(error)),
      Err(_) => return Err(FFIError::Other("scope worker exited before ready".to_string()))
    }

    *self.worker.lock().expect("worker mutex poisoned") = Some(handle);
    *txSlot = Some(tx);
    Ok(())
  }

  /// Выполняет `body` на потоке-владельце клона и возвращает результат.
  /// Безопасно звать с любого потока хоста. Один клон — одна операция за раз
  /// (сокет IPC клона один; очередь сериализует вызовы).
  pub fn withScope<R>(&self, body: impl FnOnce(&FFIScope) -> R + Send + 'static) -> Result<R, FFIError>
  where
    R: Send + 'static
  {
    self.ensureWorker()?;

    let (replyTx, replyRx) = mpsc::channel::<Result<(R, Option<i32>), String>>();
    {
      let txSlot: MutexGuard<'_, Option<Sender<WorkerMsg>>> =
        self.tx.lock().expect("scope mutex poisoned");
      if self.closed.load(Ordering::SeqCst)
      {
        return Err(FFIError::Other("scope is closed".to_string()));
      }
      let tx: Sender<WorkerMsg> = txSlot
        .clone()
        .ok_or_else(|| FFIError::Other("scope worker is not running".to_string()))?;

      tx.send(WorkerMsg::Run(Box::new(move |ffiScope: &FFIScope|
      {
        let outcome: Result<(R, Option<i32>), String> =
          match std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
          {
            let value: R = body(ffiScope);
            let errno: Option<i32> = ChillScope::lastErrno();
            (value, errno)
          }))
          {
            Ok(pair) => Ok(pair),
            Err(payload) => Err(panicPayload(payload))
          };
        let _ = replyTx.send(outcome);
      }))).map_err(|_| FFIError::Other("scope worker stopped".to_string()))?;
    }

    match replyRx.recv()
    {
      Ok(Ok((value, errno))) =>
      {
        *self.lastErrno.lock().expect("errno mutex poisoned") = errno;
        Ok(value)
      }
      Ok(Err(message)) => Err(FFIError::Other(message)),
      Err(_) => Err(FFIError::Other("scope worker dropped the reply".to_string()))
    }
  }

  /// scope-уровень errno-политики (`Scope::setReadErrno` в `chillffi`).
  pub fn setReadErrno(&self, enabled: bool) -> ()
  {
    *self.readErrno.lock().expect("errno mutex poisoned") = Some(enabled);
  }

  /// Чтение уровня скоупа для резолва политики в `PyCall::finish`:
  /// `None` — не переопределён, решает глобальный дефолт
  /// (`chillffi.setGlobalReadErrno`, `errnoPolicy.rs`).
  pub fn readErrno(&self) -> Option<bool>
  {
    *self.readErrno.lock().expect("errno mutex poisoned")
  }

  /// Errno последней операции на клоне этого скоупа (не thread-local вызывающего).
  pub fn lastErrno(&self) -> Option<i32>
  {
    *self.lastErrno.lock().expect("errno mutex poisoned")
  }

  pub fn isOpen(&self) -> bool
  {
    !self.closed.load(Ordering::SeqCst)
  }

  /// Канал закрывается, владелец дропает клон и join'ится.
  fn exitBlock(&self) -> ()
  {
    let tx: Option<Sender<WorkerMsg>> =
    {
      let mut txSlot: MutexGuard<'_, Option<Sender<WorkerMsg>>> =
        self.tx.lock().expect("scope mutex poisoned");
      if self.closed.swap(true, Ordering::SeqCst)
      {
        return;
      }
      txSlot.take()
    };
    if let Some(tx) = tx
    {
      let _ = tx.send(WorkerMsg::Shutdown);
    }
    if let Some(handle) = self.worker.lock().expect("worker mutex poisoned").take()
    {
      let _ = handle.join();
    }
  }
}

// todo desc
fn panicPayload(payload: Box<dyn std::any::Any + Send>) -> String
{
  if let Some(message) = payload.downcast_ref::<&str>()
  {
    return (*message).to_string();
  }
  if let Some(message) = payload.downcast_ref::<String>()
  {
    return message.clone();
  }
  "scope worker panicked".to_string()
}

// =================================================================================================
// Scope (python)

/// FFI-скоуп — аналог `ffi! { |scope| ... }` из `chillffi`, но с границами,
/// неизвестными на момент компиляции: границы задаёт хост-интерпретатор
/// (`with`-блок).
///
/// Клон привязан к потоку-владельцу внутри скоупа, не к потоку Python,
/// создавшему `Scope`. Вызовы с других потоков и из `asyncio.to_thread` идут
/// в тот же клон.
///
/// ```python
/// with chillffi.Scope() as scope:
///     lib = scope.load("libm.so.6")
///     result = lib.sqrt.arg(16.0).result("f64")   # 4.0
/// ```
#[pyclass(name = "Scope")]
pub struct Scope
{
  slot: Arc<ScopeSlot>
}

#[pymethods]
impl Scope
{
  /// Вход в блок: кладёт пустой (ленивый) слот. Ничего тяжёлого:
  /// ни зигота, ни клон не поднимаются — это делает сам `chillffi` при первом вызове.
  #[new]
  fn new() -> Self
  {
    Scope { slot: Arc::new(ScopeSlot::new()) }
  }

  // todo desc
  fn __enter__(slf: Py<Self>) -> Py<Self>
  {
    slf
  }

  /// Слот снимается, клон умирает по RAII на потоке-владельце.
  fn __exit__(&self, _excType: &Bound<'_, PyAny>, _exc: &Bound<'_, PyAny>, _tb: &Bound<'_, PyAny>) -> PyResult<()>
  {
    self.slot.exitBlock();
    Ok(())
  }

  /// Явное закрытие без `with`. Повторный вызов безопасен.
  fn close(&self) -> ()
  {
    self.slot.exitBlock();
  }

  /// Проброс `Scope::addSearchPath` — локальный путь поиска скоупа.
  fn addSearchPath(&self, py: Python<'_>, path: String) -> PyResult<()>
  {
    self.checkOpen()?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    py.detach(move ||
    {
      slot.withScope(move |ffiScope: &FFIScope|
      {
        let scope: ChillScope<'_> = ffiScope.scope();
        scope.addSearchPath(path);
      })
    }).map_err(toPyErr)
  }

  /// Проброс `Scope::setReadErrno` — scope-уровень errno-политики: каждый
  /// вызов скоупа захватывает errno (если вызов не перебил флаг сам через
  /// `.errno()`/`.noErrno()`). Захваченный errno — `scope.lastErrno()`.
  fn setReadErrno(&self, enabled: bool) -> ()
  {
    self.slot.setReadErrno(enabled);
  }

  /// Errno последней операции этого скоупа (клон один — значение общее,
  /// не привязано к потоку Python, который читает).
  fn lastErrno(&self) -> Option<i32>
  {
    self.slot.lastErrno()
  }

  /// `chillffi` `Scope::alloc` — блок памяти в куче клона (`AllocatedMemory`):
  /// данные живут в клоне, пишутся/читаются через IPC
  /// (`writeMemory`/`readMemory`). Освобождается при сборке объекта или
  /// закрытии скоупа (`Scope::free`).
  fn alloc(&self, py: Python<'_>, length: usize) -> PyResult<PyMemory>
  {
    self.checkOpen()?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    let (address, realLength): (usize, usize) = py.detach(move ||
    {
      slot.withScope(move |ffiScope: &FFIScope|
      {
        let scope: ChillScope<'_> = ffiScope.scope();
        let mem = scope.alloc(length)?;
        let pair: (usize, usize) = (mem.address(), mem.length());
        // `AllocatedMemory::drop` отправил бы Free немедленно — RAII переносим
        // на `PyMemory::drop` (free по сборке объекта / закрытию скоупа).
        std::mem::forget(mem);
        Ok(pair)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)?;
    Ok(PyMemory { slot: self.slot.clone(), address, length: realLength })
  }

  /// `chillffi` `Scope::allocStruct` — блок памяти клона под структуру с
  /// динамической формой (`[chillffi.Type...]`). Размер не считается на
  /// вызывающем: его считает клон той же ABI-математикой `libffi`
  /// (`ffi_get_struct_offsets` — padding, выравнивание, вложенные формы),
  /// как для `readDynamicStruct`/`writeDynamicStruct`.
  ///
  /// ```python
  /// mem = scope.allocStruct([chillffi.Type.I32, chillffi.Type.Pointer])
  /// ```
  fn allocStruct(&self, py: Python<'_>, fields: &Bound<'_, PyAny>) -> PyResult<PyMemory>
  {
    self.checkOpen()?;
    let shape: Vec<FfiType> = shapeFromPy(fields)?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    let (address, realLength): (usize, usize) = py.detach(move ||
    {
      slot.withScope(move |ffiScope: &FFIScope|
      {
        let scope: ChillScope<'_> = ffiScope.scope();
        let mem = scope.allocStruct(&shape)?;
        let pair: (usize, usize) = (mem.address(), mem.length());
        // RAII переносим на `PyMemory::drop` — как в `alloc` выше.
        std::mem::forget(mem);
        Ok(pair)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)?;
    Ok(PyMemory { slot: self.slot.clone(), address, length: realLength })
  }

  /// `chillffi` `Scope::readDynamicStruct` (статический в Rust) — читает
  /// структуру по адресу клона под формой из рантайма. Форма объявляет
  /// вызывающий — поля извлекаются тем типом, под которым их прочитал клон.
  /// Возвращает `chillffi.StructValue`: `out.get(0)`, вложенное —
  /// `out.getStruct(0)`, размер — `len(out)`.
  ///
  /// ```python
  /// fields = scope.readDynamicStruct(mem.address(), [chillffi.Type.I64, chillffi.Type.I64])
  /// secs, nanos = fields.get(0), fields.get(1)
  /// ```
  fn readDynamicStruct(
    &self,
    py: Python<'_>,
    pointer: &Bound<'_, PyAny>,
    fields: &Bound<'_, PyAny>
  ) -> PyResult<Py<PyAny>>
  {
    self.checkOpen()?;
    let address: usize = addressValue(pointer)?;
    let shape: Vec<FfiType> = shapeFromPy(fields)?;
    let shapeForCall: Vec<FfiType> = shape.clone();
    let slot: Arc<ScopeSlot> = self.slot.clone();
    let list = py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::readDynamicStruct(address, &shapeForCall)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)?;
    let structValue: FfiStructValue = FfiStructValue::from(list);
    structValueToPython(py, &structValue, &shape)
  }

  /// `chillffi` `Scope::writeDynamicStruct` — пишет значения полей в структуру
  /// по адресу клона под формой из рантайма. Значения — список аргументов
  /// (те же, что в `.arg()`); поле-указатель — `chillffi.Pointer(...)` с
  /// адресом клона; ширина значения должна совпадать с типом поля
  /// (`chillffi.I32(...)` под `Type.I32` — голый int ушёл бы как i64).
  ///
  /// ```python
  /// scope.writeDynamicStruct(data.address(), shape, [chillffi.I32(len(values)), chillffi.Pointer(valuesMem)])
  /// ```
  fn writeDynamicStruct(
    &self,
    py: Python<'_>,
    pointer: &Bound<'_, PyAny>,
    fields: &Bound<'_, PyAny>,
    values: &Bound<'_, PyAny>
  ) -> PyResult<()>
  {
    self.checkOpen()?;
    let address: usize = addressValue(pointer)?;
    let shape: Vec<FfiType> = shapeFromPy(fields)?;
    let args: Vec<chillffi::ffi::types::primitive::Arg> = argsFromPy(values)?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::writeDynamicStruct(address, &shape, args)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)
  }

  /// `chillffi` `Scope::readMemory` (статический в Rust) — читает `length`
  /// байт по произвольному адресу клона. Для блоков `scope.alloc` есть
  /// удобный `Memory.read()`; этот метод — про адреса, вернувшиеся из C
  /// (поле-указатель структуры, calloc, ...).
  ///
  /// ```python
  /// valuesPtr = fields.get(1)  # int *values из struct Data
  /// bytesOut = scope.readMemory(valuesPtr, size * 4)
  /// ```
  fn readMemory(&self, py: Python<'_>, pointer: &Bound<'_, PyAny>, length: usize) -> PyResult<Py<PyAny>>
  {
    self.checkOpen()?;
    let address: usize = addressValue(pointer)?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    let bytes: Vec<u8> = py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::readMemory(address, length)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)?;
    Ok(PyBytes::new(py, &bytes).into_any().unbind())
  }

  /// `chillffi` `Scope::writeMemory` (статический в Rust) — пишет байты по
  /// произвольному адресу клона. Для блоков `scope.alloc` есть `Memory.write()`.
  fn writeMemory(&self, py: Python<'_>, pointer: &Bound<'_, PyAny>, data: Vec<u8>) -> PyResult<()>
  {
    self.checkOpen()?;
    let address: usize = addressValue(pointer)?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::writeMemory(address, data)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)
  }

  /// `chillffi` `Scope::free` (статический в Rust) — освобождает блок по
  /// адресу: аллокация C (`strdup`, `malloc`), на которую у нас нет `Memory`
  /// и RAII. Блоки `scope.alloc`/`allocStruct` освобождаются сами
  /// (`Memory.drop`), явный `free` — для чужих аллокаций.
  fn free(&self, py: Python<'_>, pointer: &Bound<'_, PyAny>) -> PyResult<()>
  {
    self.checkOpen()?;
    let address: usize = addressValue(pointer)?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    py.detach(move ||
    {
      slot.withScope(move |_ffiScope: &FFIScope|
      {
        ChillScope::free(address)
      })
    }).map_err(toPyErr)?.map_err(toPyErr)
  }

  /// `chillffi` `Scope::callback` + `callback!` — питоновская функция как
  /// C function pointer. Тело уезжает в клон байтами (marshal/pickle) и
  /// исполняется тем же интерпретатором, что поднял зиготу, — там же,
  /// где живут данные. Сигнатура: "i32(pointer,pointer)", токены как у
  /// `.result()`: `i8..i64|isize|u8..u64|usize|f32|f64|bool|pointer|void`.
  /// Подробности — в `examples/callback/main.py`.
  fn callback(&self, py: Python<'_>, signature: String, callbackFn: Bound<'_, PyAny>) -> PyResult<PyCallback>
  {
    self.checkOpen()?;
    let (argTypes, returnType): (Vec<(u8, FfiType)>, (u8, FfiType)) = parseCallbackSignature(&signature)?;
    let sendable: Sendable = buildCallbackSendable(py, &callbackFn, argTypes, returnType)?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    let handle: FfiCallback = py.detach(move ||
    {
      slot.withScope(move |ffiScope: &FFIScope|
      {
        let scope: ChillScope<'_> = ffiScope.scope();
        scope.callback(sendable)
      })
    }).map_err(toPyErr)?;
    Ok(PyCallback { slot: self.slot.clone(), handle, signature })
  }

  /// Старт цепочки вызова по адресу — `chillffi` `Scope::callPointer`, без
  /// `dlopen`/`dlsym`. Адрес живёт в пространстве клона: то, что вернул
  /// предыдущий вызов C (`signal()` возвращает прежний handler, `calloc` —
  /// адрес блока), или `Memory`/`chillffi.Pointer`. Адреса хоста в клоне
  /// невалидны (изоляция процессов) — позвать из питона напрямую нельзя.
  ///
  /// Возвращает тот же `CallBuilder`, что и `lib.имя`:
  ///
  /// ```python
  /// scope.pointer(addr).arg(x).result("i32")      # ~ Scope::callPointer::<i32>
  /// scope.pointer(addr).arg(x).errno().result("i32")  # ~ Scope::callPointerErrno
  /// scope.pointer(addr).arg(x).result()           # ~ Scope::callvPointer (void)
  /// ```
  ///
  /// errno читается `scope.lastErrno()`. Вариативной формы у адресных
  /// вызовов в `chillffi` нет — `.variadic()` у Pointer-цели отвергается.
  fn pointer(&self, address: &Bound<'_, PyAny>) -> PyResult<PyCall>
  {
    self.checkOpen()?;
    Ok(PyCall::pointer(self.slot.clone(), addressValue(address)?))
  }

  /// Проброс `Scope::load`: резолвит путь (локальный resolver скоупа → глобальный
  /// → как дано) и возвращает хэндл библиотеки, привязанный к этому скоупу.
  /// Сам dlopen не происходит ни здесь, ни вообще на стороне хоста: клон
  /// загружает библиотеку лениво при первом вызове и кэширует по пути.
  fn load(&self, py: Python<'_>, libraryPath: String) -> PyResult<PyLib>
  {
    self.checkOpen()?;
    let slot: Arc<ScopeSlot> = self.slot.clone();
    let resolved: String = py.detach(move ||
    {
      slot.withScope(move |ffiScope: &FFIScope|
      {
        let scope: ChillScope<'_> = ffiScope.scope();
        let lib: Library<'_> = scope.load(&libraryPath)?;
        Ok(lib.path().to_string())
      })
    }).map_err(toPyErr)?.map_err(toPyErr)?;
    Ok(PyLib { slot: self.slot.clone(), path: resolved, unloaded: Arc::new(AtomicBool::new(false)) })
  }
}

impl Scope
{
  // todo desc
  fn checkOpen(&self) -> PyResult<()>
  {
    if self.slot.isOpen() { Ok(()) }
    else
    {
      Err(PyRuntimeError::new_err(
        "scope is closed (the `with` block has exited or close() was called)"
      ))
    }
  }
}

/// Адрес в пространстве клона для `scope.pointer(addr)`:
/// `int` (то, что вернул C — `signal()`/`calloc`, или `Memory.address()`),
/// либо сам `Memory`/`chillffi.Pointer`. Голый int — как `impl Into<usize>`
/// в Rust: валидность адреса — на вызывающем (изоляция переживёт промах).
fn addressValue(object: &Bound<'_, PyAny>) -> PyResult<usize>
{
  if let Ok(v) = object.extract::<usize>() { return Ok(v); }
  if let Ok(m) = object.cast::<PyMemory>() { return Ok(m.borrow().address); }
  if let Ok(p) = object.cast::<TypedPointer>() { return Ok(p.borrow().address); }
  Err(PyRuntimeError::new_err(format!(
    "address: ожидаю int (адрес из пространства клона), Memory или chillffi.Pointer, got {}",
    object.get_type().name()?
  )))
}

impl Drop for Scope
{
  /// Блок, убранный GC без __exit__, закрывается так же.
  fn drop(&mut self) -> ()
  {
    self.slot.exitBlock();
  }
}

// =================================================================================================
