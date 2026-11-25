"""
paths — зеркало chillffi/examples/paths/main.rs.

Резолюция библиотек: прямой путь, путь поиска скоупа, глобальный путь поиска
(общий на процесс — `chillffi.addGlobalSearchPath`).

C-помощник собирает `run.rs` (запуск примера) -> `examples/paths/libprint.so`

Запуск: python3 examples/paths/main.py
"""
# ======================================================================================================================

import os
import sys

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import platformExt  # noqa: E402

PathsDir: str = os.path.join(TestDir, "paths")

# ======================================================================================================================
# Путь с '/' — идёт сразу в `dlopen`, `PathResolver` пропускается.
# В Rust-примере путь относительный ("./examples/paths/libprint") — работает
# только из корня репозитория; здесь абсолютный путь из `__file__`, чтобы тест
# не зависел от текущего каталога.

def rawPath() -> None:
  rawLibrary: str = os.path.join(PathsDir, platformExt("libprint"))

  with chillffi.Scope() as scope:
    libprint: chillffi.Library = scope.load(rawLibrary)
    result: int = libprint.print.arg(b"raw path\n").arg(9).result("usize")

  assert result == 0, f"print() != NULL ({result})"
  print("ok: raw path")

# ======================================================================================================================
# Путь поиска скоупа — виден только внутри этого блока.

def scopePath() -> None:
  with chillffi.Scope() as scope:
    scope.addSearchPath(PathsDir)
    libprint: chillffi.Library = scope.load(platformExt("libprint"))
    result: int = libprint.print.arg(b"scope path\n").arg(10).result("usize")

  assert result == 0, f"print() != NULL ({result})"
  print("ok: scope path")

# ======================================================================================================================
# Глобальный путь поиска — виден во всех блоках, общий на процесс.
# Rust: `chillffi::pathResolver::addGlobalSearchPath("examples/paths")`;
# `chillffi.addGlobalSearchPath(dir)` — общий на процесс список каталогов,
# виден всем скоупам (включая уже созданные и будущие). Резолв выполняет
# само ядро в `scope.load("имя")`, как в `chillffi`: локальный путь скоупа
# -> глобальный -> путь как дан.

def globalPath() -> None:
  chillffi.addGlobalSearchPath(PathsDir)

  # Скоуп без локального `addSearchPath` — имя резолвится глобальным списком.
  with chillffi.Scope() as scope:
    libprint: chillffi.Library = scope.load(platformExt("libprint"))
    result: int = libprint.print.arg(b"global path\n").arg(11).result("usize")

  assert result == 0, f"print() != NULL ({result})"
  print("ok: global path")

# ======================================================================================================================

def main() -> None:
  rawPath()
  scopePath()
  globalPath()

if __name__ == "__main__":
  main()

# ======================================================================================================================
