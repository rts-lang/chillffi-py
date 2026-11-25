"""
Платформенные данные для тестов chillffi-py — зеркало chillffi/examples/platform/mod.rs.

Rust-версия лежит в `examples/platform/mod.rs` (общий каталог примеров и тестов).
Здесь то же самое, но на питоне.

Важно про имя файла: каталог называется `platform` (как в `chillffi`), а модуль —
`platformData`, а не `platform`: иначе `import platform` перекрыл бы стандартный
модуль питона `platform` (его импортирует половина экосистемы). Поэтому
`tests/platform` кладётся в `sys.path` и импортируется как `platformData`.

Использование в тестах:

    import os, sys
    TESTS_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    sys.path.insert(0, os.path.join(TESTS_DIR, "platform"))
    from platformData import LibcPath, LibmPath, platformExt
"""
# ======================================================================================================================

import sys

# ======================================================================================================================
# Платформа

# Rust-макрос `platformExt!("libX")` -> `libX.so` / `libX.dylib` / `libX.dll`.

def platformExt(name):
  if sys.platform == "darwin":
    return name + ".dylib"
  if sys.platform == "win32":
    return name + ".dll"
  return name + ".so"

# ======================================================================================================================
# Linux

class Linux:
  IsLinux = True
  IsMacos = False
  IsWindows = False

  LibcPath = "libc.so.6"
  LibmPath = "libm.so.6"

  # stat
  StatSize = 144
  StSizeOffset = 48
  EtcHostnameString = "/etc/hostname"
  EtcHostnameCString = "/etc/hostname"
  StatSymbolName = "stat"

  OpenSymbolName = "open"
  CloseSymbolName = "close"
  ReadSymbolName = "read"
  WriteSymbolName = "write"
  PipeSymbolName = "pipe"
  StrdupSymbolName = "strdup"

  TimeLibPath = LibcPath
  TimeSymbolName = "clock_gettime"

  SignalNumber = 10  # SIGUSR1

  SprintfLibPath = LibcPath
  SprintfSymbolName = "sprintf"

  # clock_gettime(CLOCK_REALTIME, struct timespec *) — первый аргумент
  ClockRealtime = 0

# ======================================================================================================================
# macOS

class Macos:
  IsLinux = False
  IsMacos = True
  IsWindows = False

  LibcPath = "libSystem.dylib"
  LibmPath = "libSystem.dylib"

  # stat
  StatSize = 144
  StSizeOffset = 96
  EtcHostnameString = "/etc/hosts"
  EtcHostnameCString = "/etc/hosts"
  # На Intel у stat — 64-битный вариант, на arm64 — обычный.
  StatSymbolName = "stat$INODE64" if sys.maxsize <= 2**32 else "stat"

  OpenSymbolName = "open"
  CloseSymbolName = "close"
  ReadSymbolName = "read"
  WriteSymbolName = "write"
  PipeSymbolName = "pipe"
  StrdupSymbolName = "strdup"

  TimeLibPath = LibcPath
  TimeSymbolName = "clock_gettime"

  SignalNumber = 10  # SIGUSR1

  SprintfLibPath = LibcPath
  SprintfSymbolName = "sprintf"

  ClockRealtime = 0

# ======================================================================================================================
# Windows

class Windows:
  IsLinux = False
  IsMacos = False
  IsWindows = True

  # ucrtbase.dll — тот же CRT, с которым собран сам тулчейн Rust, поэтому
  # errno, который видит chillffi, и errno этих функций — один и тот же.
  LibcPath = "ucrtbase.dll"
  LibmPath = "ucrtbase.dll"

  # struct _stat64 — 56 байт, st_size по смещению 24.
  StatSize = 56
  StSizeOffset = 24
  EtcHostnameString = "C:\\Windows\\System32\\drivers\\etc\\hosts"
  EtcHostnameCString = "C:\\Windows\\System32\\drivers\\etc\\hosts"
  StatSymbolName = "_stat64"

  # UCRT экспортирует POSIX-подобные вызовы с подчёркиванием.
  OpenSymbolName = "_open"
  CloseSymbolName = "_close"
  ReadSymbolName = "_read"
  WriteSymbolName = "_write"
  PipeSymbolName = "_pipe"
  StrdupSymbolName = "_strdup"

  # В UCRT нет clock_gettime — GetSystemTimeAsFileTime пишет FILETIME
  # (u64 тиков по 100нс от 1601-01-01) вместо timespec из двух полей.
  TimeLibPath = "kernel32.dll"
  TimeSymbolName = "GetSystemTimeAsFileTime"
  FileTimeUnixEpoch = 116444736000000000

  # SIGUSR1 тут нет; UCRT принимает SIGTERM.
  SignalNumber = 15  # SIGTERM

  SprintfLibPath = "ntdll.dll"
  SprintfSymbolName = "sprintf"

  ClockRealtime = 0

# ======================================================================================================================
# Выбор платформы

if sys.platform.startswith("linux"):
  Current = Linux
elif sys.platform == "darwin":
  Current = Macos
elif sys.platform in ("win32", "cygwin"):
  Current = Windows
else:
  raise RuntimeError(f"chillffi-py tests: unsupported platform '{sys.platform}'")

# Константы наружу — как `use crate::platform::...` в Rust-примерах.
LibcPath = Current.LibcPath
LibmPath = Current.LibmPath

StatSize = Current.StatSize
StSizeOffset = Current.StSizeOffset
EtcHostnameString = Current.EtcHostnameString
EtcHostnameCString = Current.EtcHostnameCString
StatSymbolName = Current.StatSymbolName

OpenSymbolName = Current.OpenSymbolName
CloseSymbolName = Current.CloseSymbolName
ReadSymbolName = Current.ReadSymbolName
WriteSymbolName = Current.WriteSymbolName
PipeSymbolName = Current.PipeSymbolName
StrdupSymbolName = Current.StrdupSymbolName

TimeLibPath = Current.TimeLibPath
TimeSymbolName = Current.TimeSymbolName
ClockRealtime = Current.ClockRealtime

SignalNumber = Current.SignalNumber

SprintfLibPath = Current.SprintfLibPath
SprintfSymbolName = Current.SprintfSymbolName

if Current.IsWindows:
  FileTimeUnixEpoch = Current.FileTimeUnixEpoch
else:
  FileTimeUnixEpoch = None

# ======================================================================================================================
