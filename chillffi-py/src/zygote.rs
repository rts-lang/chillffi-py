// =================================================================================================

/// Entry зиготы.
///
/// Экспортируется наружу специально: `#[unsafe(no_mangle)]`.
/// 
/// Вызывается из Python-лаунчера:
///
/// `<sys.executable> -I -c "import ctypes,sys; ctypes.CDLL(<этот .so>).chillffiRunZygote(sys.argv[1].encode())"`.
///
/// `name`: Имя `IpcOneShotServer`: IPC-эндпоинт, который слушает `runtime` и к которому
/// подключается зигота. Приходит из `argv[1]`; C-строка, non-null, NUL-terminated.
///
/// Не возвращает: Уходит в control loop зиготы (`chillffi::runAsZygoteNamed` → `!`).
#[unsafe(no_mangle)]
pub extern "C" fn chillffiRunZygote(name: *const std::os::raw::c_char) -> ()
{
  let serverName: String = unsafe{ std::ffi::CStr::from_ptr(name) }
    .to_string_lossy()
    .into_owned();
  chillffi::runAsZygoteNamed(serverName)
}

// =================================================================================================
