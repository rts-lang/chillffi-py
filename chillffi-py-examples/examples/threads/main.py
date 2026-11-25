"""
threads — scope не привязан к потоку Python, который его создал.

Клон живёт на потоке-владельце внутри Scope. Вызовы с других потоков и
из asyncio.to_thread сериализуются в тот же клон (один IPC-сокет).
Два Scope — два клона, их можно звать параллельно.

Запуск: python3 examples/threads/main.py
"""
# ======================================================================================================================

import asyncio
import os
import sys
from concurrent.futures import ThreadPoolExecutor
from threading import Barrier

TestDir: str = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(TestDir, "platform"))

import chillffi  # noqa: E402
from platformData import LibcPath, LibmPath  # noqa: E402

# ======================================================================================================================
# `Scope` создан на main; `getpid` с worker-потока — тот же pid клона.

def sameCloneFromOtherThread() -> None:
  with chillffi.Scope() as scope:
    libc: chillffi.Library = scope.load(LibcPath)
    mainPid: int = libc.getpid.result("i32")

    def other() -> int:
      return libc.getpid.result("i32")

    with ThreadPoolExecutor(max_workers=1) as pool:
      otherPid: int = pool.submit(other).result()

  assert mainPid == otherPid and mainPid > 0, f"{mainPid} != {otherPid}"
  assert mainPid != os.getpid(), "clone pid must differ from host"
  print("ok: same clone from another thread, pid =", mainPid)

# Несколько потоков бьют в один `Scope` — очередь, один клон, ответы корректны.

def concurrentCallsOneScope() -> None:
  with chillffi.Scope() as scope:
    libm: chillffi.Library = scope.load(LibmPath)

    def worker(value: float) -> float:
      return libm.sqrt.arg(value).result("f64")

    with ThreadPoolExecutor(max_workers=4) as pool:
      futures = [pool.submit(worker, 4.0) for _ in range(8)]
      results = [f.result() for f in futures]

    for result in results:
      assert abs(result - 2.0) < 1e-12, f"sqrt(4.0) != 2.0 ({result})"

    clonePid: int = scope.load(LibcPath).getpid.result("i32")
  print("ok: 8 calls / 4 threads, one clone pid =", clonePid)

# `asyncio.to_thread` уводит вызов в пул; клон тот же.

def asyncioToThread() -> None:
  async def run() -> None:
    with chillffi.Scope() as scope:
      libc: chillffi.Library = scope.load(LibcPath)

      def call() -> int:
        return libc.abs.arg(-5).result("i32")

      result: int = await asyncio.to_thread(call)
      assert result == 5, f"abs(-5) != 5 ({result})"
      clonePid: int = await asyncio.to_thread(lambda: libc.getpid.result("i32"))
      assert clonePid > 0
      print("ok: asyncio.to_thread, clone pid =", clonePid)

  asyncio.run(run())

# Два `Scope` — два клона: `Barrier` сводит их к одновременному старту,
# вызовы не сериализуются друг с другом.

def twoScopesParallel() -> None:
  barrier = Barrier(2)

  def run(value: float) -> tuple[float, int]:
    with chillffi.Scope() as scope:
      libm: chillffi.Library = scope.load(LibmPath)
      libc: chillffi.Library = scope.load(LibcPath)
      barrier.wait()
      root: float = libm.sqrt.arg(value).result("f64")
      pid: int = libc.getpid.result("i32")
      return root, pid

  with ThreadPoolExecutor(max_workers=2) as pool:
    a = pool.submit(run, 4.0)
    b = pool.submit(run, 9.0)
    rootA, pidA = a.result()
    rootB, pidB = b.result()

  assert abs(rootA - 2.0) < 1e-12
  assert abs(rootB - 3.0) < 1e-12
  assert pidA != pidB, f"expected distinct clones, got {pidA} == {pidB}"
  print("ok: two scopes in parallel, pids =", pidA, pidB)

# ======================================================================================================================

def main() -> None:
  sameCloneFromOtherThread()
  concurrentCallsOneScope()
  asyncioToThread()
  twoScopesParallel()


if __name__ == "__main__":
  main()

# ======================================================================================================================
