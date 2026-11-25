// =================================================================================================
// examples/run.rs — единственный `.rs`-файл для всех примеров.
//
// Примеров-таргетов у cargo не будет: ни одного `examples/<имя>.rs` и ни одного
// `examples/<имя>/main.rs`. Все примеры — это питон-скрипты `examples/<имя>/main.py`,
// и запускаются они отсюда.
//
// Запуск:
//
//   cargo ex <имя>                        — один пример;
//   cargo run --package chillffi-py-examples --example run -- <имя>
//
// Алиас `ex` (в `.cargo/config.toml`) раскрывается в
// `run --package chillffi-py-examples --example run --`, поэтому имя примера
// приходит в `argv[1]`.
//
// Логика (перенесена из `tests/suite.rs`):
//
//   main() -> prepare()   // один раз: C-помощники, venv, модуль
//          -> runScript() // пример на питоне из venv
//
// Почему не `build.rs`.
//
//   `cargo build` и `cargo run` должны вести себя по-разному: сборка — быстро и
//   чисто, запуск примера — с полным окружением. Штатного способа отличить их
//   внутри `build.rs` нет:
//
//     * `#[cfg(test)]` в `build.rs` не работает: Cargo компилирует build-скрипт
//       обычным бинарником и никогда — с `--test`, поэтому `cfg(test)` там всегда false;
//     * `PROFILE` = "debug" и при `cargo build`, и при `cargo run` (Cargo Book:
//       "PROFILE — release for release builds, debug for other builds");
//     * `CARGO_CFG_TEST` не существует (Cargo Book: "Some cfg values like test are not
//       available").
//
//   Поэтому всё окружение готовится в рантайме: обычная сборка остаётся чистой.
//
// Где лежит `venv` и как собирается модуль.
//
//   * `venv` — `<корень воркспейса>/.venv` (то есть рядом с `Cargo.toml` воркспейса);
//   * примеры с C-исходниками компилируются в общие библиотеки;
//   * `cdylib` (`cargo run` его не собирает — `crate-type = ["cdylib"]`) берётся
//     отдельным вложенным `cargo build --lib --package chillffi-py` в общий
//     `target-dir` воркспейса: так переиспользуются уже собранные зависимости.
// =================================================================================================

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};

// =================================================================================================
// Harness

/// Имя пакета основного крейта — его `cdylib` собирает обвязка и ставит в `venv`.
const MainPackage: &str = "chillffi-py";

fn main() -> ExitCode
{
  // 1. Имя примера — из `argv[1]`.
  let name: String = match env::args().nth(1)
  {
    Some(name) if !name.is_empty() => name,
    _ =>
    {
      eprintln!("usage: cargo ex <example-name>");
      return ExitCode::from(2);
    }
  };

  println!("example: {name}");

  // 2. Подготовка окружения. Ошибка подготовки — пример не запустится.
  if let Err(error) = prepare()
  {
    eprintln!("Setup failed: {error}");
    return ExitCode::FAILURE;
  }

  // 3. Сам пример.
  let script: PathBuf = examplesDir().join(&name).join("main.py");

  if !script.exists()
  {
    eprintln!("Нет примера '{name}': {}", script.display());
    return ExitCode::FAILURE;
  }

  match runScript(&script)
  {
    Ok(()) => ExitCode::SUCCESS,
    Err(error) =>
    {
      eprintln!("{error}");
      ExitCode::FAILURE
    }
  }
}

/// Глобальная подготовка: всё, что нужно примеру, но не нужно обычной сборке.
fn prepare() -> Result<(), String>
{
  // 1. C-помощники для примеров (как в `chillffi/build.rs`).
  compileExamples(&examplesDir())?;

  // 2. `virtualenv` в корне воркспейса.
  let venvPython: PathBuf = ensureVenv()?;

  // 3. Модуль: собрать `cdylib` и поставить его в `venv`.
  //    Вложенная сборка ничего не запускает — рекурсии нет.
  let module: PathBuf = buildModule()?;
  installModule(&venvPython, &module)?;

  // 4. Маркер подготовки: видно, что окружение живое.
  writeMarker()?;

  Ok(())
}

// =================================================================================================
// Окружение: пути

/// Корень этого крейта (`chillffi-py-examples`).
fn ownDir() -> PathBuf
{
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Корень воркспейса — на уровень выше (`Cargo.toml` воркспейса, общий `target/`).
fn workspaceDir() -> PathBuf
{
  ownDir().parent().map(|parent| parent.to_path_buf()).unwrap_or_else(ownDir)
}

/// Каталог с примерами/тестами.
fn examplesDir() -> PathBuf
{
  ownDir().join("examples")
}

/// `virtualenv`: `<корень воркспейса>/.venv`.
fn venvDir() -> PathBuf
{
  workspaceDir().join(".venv")
}

/// Служебный каталог сборки примеров и тестов: `<корень воркспейса>/target/py-suite`.
fn suiteDir() -> PathBuf
{
  workspaceDir().join("target").join("py-suite")
}

/// Интерпретатор внутри `virtualenv` (Unix / Windows).
fn venvPython(venv: &Path) -> PathBuf
{
  if cfg!(windows) {
    venv.join("Scripts").join("python.exe")
  } else {
    venv.join("bin").join("python")
  }
}

// =================================================================================================
// Окружение: `venv`

/// Создаёт `virtualenv`, если его ещё нет, и возвращает путь к интерпретатору.
fn ensureVenv() -> Result<PathBuf, String>
{
  let venv: PathBuf = venvDir();
  let python: PathBuf = venvPython(&venv);

  if python.exists()
  {
    println!("  venv уже есть: {}", venv.display());
    return Ok(python);
  }

  println!("  создаю venv {}", venv.display());

  // Интерпретатор для создания `venv`: можно указать свой.
  let base: String =
    env::var("СhillffiPython")
      .or_else(|_| env::var("PYTHON"))
      .unwrap_or_else(|_| "python3".to_string());

  let status: std::io::Result<ExitStatus> =
    Command::new(&base).args(["-m", "venv", venv.to_str().unwrap_or("./.venv")]).status();

  if !matches!(status, Ok(status) if status.success()) || !python.exists()
  {
    return Err(format!(
      "не удалось создать venv через '{base} -m venv {}' — укажи интерпретатор через СhillffiPython",
      venv.display()
    ));
  }

  Ok(python)
}

// =================================================================================================
// Окружение: модуль (`cdylib`)

/// Собирает `cdylib` крейта `chillffi-py` вложенным `cargo build --lib` и возвращает путь к `.so`/`.dll`.
///
/// Вложенная сборка идёт в тот же `target-dir` и в тот же профиль, что и внешний
/// `cargo run`. Благодаря этому зависимости (`chillffi`, `pyo3`, `libffi`,
/// proc-макросы) уже собраны внешним проходом, и вложенный build компилирует
/// только сам крейт. С отдельным `target-dir` ничего не переиспользовалось — дерево
/// собиралось дважды.
///
/// Профиль: у примера нет переменной `PROFILE` (это привилегия build-скриптов),
/// поэтому ориентируемся на debug-assertions: dev/test -> профиль "test",
/// release -> профиль "release".
fn buildModule() -> Result<PathBuf, String>
{
  let workspace: PathBuf = workspaceDir();
  let release: bool = !cfg!(debug_assertions);

  let cargo: String = env::var("CARGO").unwrap_or_else(|_| "cargo".into());

  println!(
    "  собираю модуль (cargo build --lib -p {MainPackage}, профиль {})",
    if release { "release" } else { "test" }
  );

  let mut command: Command = Command::new(&cargo);
  command
    .current_dir(&workspace)
    // Тот же `target-dir`, что и у внешнего `cargo run`: отдельный `--target-dir`
    // заставил бы cargo собрать всё дерево зависимостей заново.
    .args(["build", "--lib", "--package", MainPackage, "--message-format=json"])
    // Cargo.lock не трогаем — иначе два cargo будут драться за один файл.
    .arg("--locked");
  // Рекурсии нет: `build.rs` у примеров удалён, вложенная сборка ничего
  // не запускает.

  if release {
    command.arg("--release");
  } else {
    command.args(["--profile", "test"]);
  }

  // Jobserver родительского cargo к этому моменту уже закрыт — вложенная сборка
  // должна работать сама, иначе она встанет в ожидании свободного потока.
  command.env_remove("CARGO_MAKEFLAGS");
  command.env_remove("MAKEFLAGS");

  let output: std::io::Result<std::process::Output> = command.output();

  let Ok(output) = output else {
    return Err(format!("не удалось запустить вложенную сборку ({cargo} build --lib)"));
  };

  let stdout: String = String::from_utf8_lossy(&output.stdout).into_owned();
  let stderr: String = String::from_utf8_lossy(&output.stderr).trim().to_string();

  if !output.status.success() {
    return Err(format!("не удалось собрать модуль ({cargo} build --lib): {stderr}"));
  }

  // Путь к артефакту cargo сообщает сам — гадать про раскладку `target-dir` не надо.
  let module: PathBuf = artifactFromCargoJson(&stdout)
    .ok_or_else(|| "не удалось найти cdylib в выводе вложенной сборки".to_string())?;

  if !module.exists() {
    return Err(format!("модуль не найден после сборки: {}", module.display()));
  }

  Ok(module)
}

/// Достаёт путь к `cdylib` из JSON-вывода cargo (`--message-format=json`).
///
/// Cargo печатает по одному JSON-объекту на артефакт; нам нужен тот, где
/// `package` — `chillffi-py`, а `crate-type` — `cdylib`. Разбираем без serde: ищем
/// строковые литералы, оканчивающиеся на `.so`/`.dll`/`.dylib`.
///
/// Ориентируемся на manifest_path пакета: у path-зависимости package_id выглядит
/// как `path+file:///...#<version>` — без имени пакета.
fn artifactFromCargoJson(json: &str) -> Option<PathBuf>
{
  let manifest: String = format!("/{MainPackage}/Cargo.toml");

  for line in json.lines()
  {
    if !line.contains("\"reason\":\"compiler-artifact\"") { continue; }
    if !line.contains("\"crate_types\":[\"cdylib\"]") { continue; }
    if !line.contains(&manifest) { continue; }

    let mut rest: &str = line;

    while let Some(start) = rest.find('"')
    {
      let after: &str = &rest[start + 1..];
      let Some(end) = after.find('"') else { break; };
      let value: &str = &after[..end];
      rest = &after[end + 1..];

      if value.ends_with(".so") ||
         value.ends_with(".dll") ||
         value.ends_with(".dylib")
      {
        return Some(PathBuf::from(value));
      }
    }
  }

  None
}

/// Имя файла модуля в `site-packages`: `chillffi.so` / `chillffi.pyd`.
fn moduleFileName() -> String
{
  if cfg!(windows) {
    "chillffi.pyd".to_string()
  } else {
    "chillffi.so".to_string()
  }
}

/// Ставит модуль в `virtualenv` (то же, что `maturin develop`).
fn installModule(venvPython: &Path, module: &Path) -> Result<(), String>
{
  let output: std::io::Result<std::process::Output> =
    Command::new(venvPython).args(["-c", "import sysconfig; print(sysconfig.get_paths()['purelib'])"]).output();

  let Ok(output) = output else {
    return Err(format!("не удалось получить site-packages из {}", venvPython.display()));
  };

  if !output.status.success() {
    return Err(format!("не удалось получить site-packages из {}", venvPython.display()));
  }

  let purelib: PathBuf = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string());
  let target: PathBuf = purelib.join(moduleFileName());

  if let Some(parent) = target.parent() { let _ = fs::create_dir_all(parent); }

  if fs::copy(module, &target).is_err() {
    return Err(format!("не удалось скопировать модуль в {}", target.display()));
  }

  println!("  модуль установлен: {}", target.display());
  Ok(())
}

/// Маркер «тестовое окружение подготовлено» — `<корень воркспейса>/target/py-suite/prepared.txt`.
fn writeMarker() -> Result<(), String>
{
  let dir: PathBuf = suiteDir();

  if fs::create_dir_all(&dir).is_err() {
    return Err(format!("не удалось создать {}", dir.display()));
  }

  let marker: PathBuf = dir.join("prepared.txt");
  let content: String = format!("profile={}\n", if cfg!(debug_assertions) { "debug" } else { "release" });

  if fs::write(&marker, content).is_err() {
    return Err(format!("не удалось записать {}", marker.display()));
  }

  Ok(())
}

// =================================================================================================
// Окружение: C-помощники

/// Флаги и расширение общей библиотеки для текущей платформы.
#[cfg(not(windows))]
const fn sharedLibSpec() -> (&'static [&'static str], &'static str)
{
  #[cfg(target_os = "macos")]
  {
    (&["-dynamiclib"], "dylib")
  }
  #[cfg(not(target_os = "macos"))]
  {
    (&["-shared", "-fPIC"], "so")
  }
}

/// Компилирует все `.c` из каталога примеров в общие библиотеки.
fn compileExamples(dir: &Path) -> Result<(), String>
{
  if !dir.exists() {
    return Ok(());
  }

  // Каталог для `.obj`/`.lib` на Windows: у примера нет `OUT_DIR` (`build.rs` удалён),
  // поэтому свой — рядом с остальными тестовыми артефактами.
  #[cfg(windows)]
  let _ = fs::create_dir_all(objDir());

  compileDir(dir)
}

/// Рекурсивный обход каталога в поисках `.c`.
fn compileDir(dir: &Path) -> Result<(), String>
{
  let Ok(entries) = fs::read_dir(dir) else {
    return Ok(());
  };

  for entry in entries.flatten()
  {
    let path: PathBuf = entry.path();

    if path.is_dir() {
      compileDir(&path)?; continue;
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("c") {
      continue;
    }

    let stem: &str = path.file_stem().unwrap().to_str().unwrap();

    #[cfg(windows)]
    let output: PathBuf = path.with_file_name(format!("lib{stem}.dll"));
    #[cfg(not(windows))]
    let output: PathBuf =
    {
      let (_, ext) = sharedLibSpec();
      path.with_file_name(format!("lib{stem}.{ext}"))
    };

    if isFresh(&path, &output) {
      continue;
    }

    #[cfg(windows)]
    let compiled: bool = compileWindows(&path, &output, stem);
    #[cfg(not(windows))]
    let compiled: bool = compileUnix(&path, &output);

    if !compiled {
      return Err(format!("не удалось скомпилировать {}", path.display()));
    }
  }

  Ok(())
}

/// Компиляция общей библиотеки Unix-ом.
#[cfg(not(windows))]
fn compileUnix(source: &Path, output: &Path) -> bool
{
  let (flags, _) = sharedLibSpec();
  let compiler: String = env::var("CC").unwrap_or_else(|_| "cc".into());
  let status: std::io::Result<ExitStatus> = Command::new(compiler)
    .args(flags)
    .arg("-o")
    .arg(output)
    .arg(source)
    .status();

  matches!(status, Ok(status) if status.success())
}

/// Компиляция общей библиотеки MSVC.
#[cfg(windows)]
fn compileWindows(source: &Path, output: &Path, stem: &str) -> bool
{
  // Отличие от `build.rs`: переменной окружения `TARGET` у примера нет (её получают
  // только build-скрипты) — тройку цели собираем на этапе компиляции.
  let target: String = targetTriple().to_string();

  // `OUT_DIR` — тоже только для build-скриптов, поэтому свой каталог.
  let outDir: PathBuf = objDir();

  let mut command: Command = cc::windows_registry::find(&target, "cl.exe")
    .unwrap_or_else(|| Command::new("cl.exe"));

  let status: std::io::Result<ExitStatus> = command
    .arg("/nologo")
    .arg("/MD")
    .arg("/LD")
    .arg(format!("/Fe:{}", output.display()))
    .arg(format!("/Fo{}{}", outDir.display(), std::path::MAIN_SEPARATOR))
    .arg(source)
    .arg("/link")
    .arg(format!("/IMPLIB:{}", outDir.join(format!("{stem}.lib")).display()))
    .status();

  matches!(status, Ok(status) if status.success())
}

/// Тройка цели для поиска `cl.exe` в реестре (`x86_64-pc-windows-msvc` и т.п.).
///
/// В `build.rs` она бралась из переменной окружения `TARGET` — у примера такой
/// переменной нет, поэтому собираем из `cfg!` на этапе компиляции.
#[cfg(windows)]
const fn targetTriple() -> &'static str
{
  if cfg!(target_env = "gnu") {
    if cfg!(target_arch = "aarch64") { "aarch64-pc-windows-gnu" }
    else { "x86_64-pc-windows-gnu" }
  }
  else if cfg!(target_arch = "aarch64") { "aarch64-pc-windows-msvc" }
  else if cfg!(target_arch = "x86") { "i686-pc-windows-msvc" }
  else { "x86_64-pc-windows-msvc" }
}

/// Каталог для объектных файлов и `.lib` на Windows: `<корень воркспейса>/target/py-suite/obj`.
#[cfg(windows)]
fn objDir() -> PathBuf
{
  suiteDir().join("obj")
}

/// Свежее ли время сборки: `.so` не старше исходника.
fn isFresh(source: &Path, output: &Path) -> bool
{
  let (Ok(sourceMeta), Ok(outputMeta)) =
    (fs::metadata(source), fs::metadata(output)) else { return false; };
  let (Ok(sourceTime), Ok(outputTime)) =
    (sourceMeta.modified(), outputMeta.modified()) else { return false; };
  outputTime >= sourceTime
}

// =================================================================================================
// Запуск примера

/// Запускает питон-скрипт примера интерпретатором из `venv`.
///
/// Вывод — на лету (`stdio` наследуется), код возврата скрипта становится кодом
/// возврата самого раннера.
fn runScript(script: &Path) -> Result<(), String>
{
  let python: PathBuf = venvPython(&venvDir());

  let status: std::io::Result<ExitStatus> =
    Command::new(&python).arg(script).current_dir(workspaceDir()).status();

  let Ok(status) = status else {
    return Err(format!("не удалось запустить {} ({})", script.display(), python.display()));
  };

  if !status.success() { return Err(format!("exit status: {status}")); }

  Ok(())
}

// =================================================================================================
