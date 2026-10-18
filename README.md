# WinSparkle-NG

A Rust reimplementation of [WinSparkle](https://github.com/vslavik/winsparkle),
providing a standalone Windows updater DLL with a compatible C API and a native
Win32 interface.

WinSparkle-NG was created using the original WinSparkle implementation as a
reference for its API, ABI, update formats, and behavior. Development is led by
**Gianluigi Tiesi** with **LLM assistance**.

## Features

- Compatible `__cdecl` C exports and the familiar `WinSparkle.dll` name.
- Native Win32 dialogs, common controls, keyboard navigation, and RTL layouts.
- Automatic Windows light/dark theme, with per-updater overrides and high-contrast support.
- WinHTTP networking with Schannel TLS and Windows proxy configuration.
- Streaming downloads, progress reporting, cancellation, and temporary-file cleanup.
- Ed25519 signature verification with strict enforcement when a trusted key is configured.
- One-click download-and-install by default, with an optional traditional two-step flow.
- Embedded translations imported from WinSparkle, with English fallback for new strings.
- Python `ctypes.CDLL` bindings and a tested Nuitka standalone packaging example.
- No wxWidgets, WebView2, OpenSSL, libcurl, or Visual C++ redistributable requirement.

## Platform support

| Target | Status |
|---|---|
| Windows 10/11 x64 (`x86_64-pc-windows-msvc`) | Built and tested |
| Windows x86 (`i686-pc-windows-msvc`) | Built; native C ABI consumer tested |
| Windows ARM64 (`aarch64-pc-windows-msvc`) | Cross-built; C ABI consumer linked; native execution still needs validation |

This is an early implementation. Authenticated enterprise proxies/PAC deployments,
high-contrast appearance, and multiple-monitor DPI behavior still need testing in
their respective environments. Reproducible checks are listed under [Testing](#testing).

## Building

Install the Rust MSVC toolchain, Visual Studio C++ Build Tools, and the Windows SDK.
From a PowerShell prompt in the repository root:

```powershell
cargo build --locked --release --target x86_64-pc-windows-msvc
./tools/package.ps1 -Arch x64 -SkipBuild
```

For a 32-bit build:

```powershell
rustup target add i686-pc-windows-msvc
./tools/package.ps1 -Arch x86
```

Packages are written to `dist/`. Each architecture ZIP contains `WinSparkle.dll`,
`WinSparkle.lib`, both C headers, the Python `ctypes` wrapper, `LICENSE`, and a
short integration README. Packaging audits the Cargo dependency graph and PE
imports/exports, and records size and resource metrics separately in `artifacts/`.

A normal build uses the committed generated resources. It does not require the
original WinSparkle checkout, gettext, or Python.

## Integration

Existing C consumers can keep using the original WinSparkle header and entry
points. Configure the updater before calling `win_sparkle_init()`, and call
`win_sparkle_cleanup()` when the application shuts down. The full contract and
intentional differences are documented in the [C API inventory](docs/api-compatibility.md).

### Installation flow

The default action is **Download and install**: a single initial confirmation
downloads, verifies when required, and launches the installer. To restore the
traditional **Download → Install now** flow, call the WinSparkle-NG extension
before initialization:

```c
win_sparkle_set_install_after_download(0);
```

Pass `1` to enable one-click installation again. The existing
`win_sparkle_check_update_with_ui_and_install()` entry point retains its original
behavior: it skips the initial update prompt entirely.

If the host vetoes shutdown or installer launch fails, the downloaded file is
retained and the dialog offers an installation retry.

### Signature policy

- **No trusted key:** unsigned updates are accepted for compatibility, with a
  diagnostic warning. A signature in the feed alone does not establish trust.
- **Trusted Ed25519 key configured:** a missing or invalid signature blocks
  installation. An invalid key configuration also fails closed.
- **Legacy DSA:** the export is retained and returns `0`. A DSA-only configuration
  blocks the download; OpenSSL is not introduced to support it.

### Light and dark themes

The default is **auto**, following the Windows application color preference.
Windows theme changes update the open updater dialog. The WinSparkle-NG extension
can override this before initialization or while the dialog is visible:

```c
win_sparkle_set_theme(WIN_SPARKLE_THEME_DARK);
win_sparkle_set_theme(WIN_SPARKLE_THEME_LIGHT);
win_sparkle_set_theme(WIN_SPARKLE_THEME_AUTO);
```

The values are `0` (auto), `1` (light), and `2` (dark). The function returns `1`
on success or `0` for an invalid value, leaving the previous mode intact.
High contrast takes precedence. Theme selection affects only updater windows,
is not persisted, and does not change the host application's theme.

The theme implementation targets Windows 10 and 11. Caption colors and native scrollbar
styles use the available Windows facilities; unsupported styling requests fall
back to native rendering on older builds.

### Python and Nuitka

The [Python wrapper](bindings/python/winsparkle.py) uses `ctypes.CDLL` and retains
callback objects for as long as the DLL can call them. Callbacks must keep Tk or
other GUI operations on the application's own UI thread.

Preview the updater for the fictional **Lumen Notes** application, with a local
feed and a harmless payload consumed by a test callback:

```powershell
cargo build --target x86_64-pc-windows-msvc
python examples/python-nuitka/demo.py --lang en
python examples/python-nuitka/demo.py --theme dark
python examples/python-nuitka/demo.py --theme light
```

The wrapper accepts `configure(..., theme='auto')` and
`ws.set_theme('dark')` (or `Theme.AUTO`, `Theme.LIGHT`, `Theme.DARK`).
One-click installation is the default in both the wrapper and the demo; use
`configure(..., one_click=False)` or `demo.py --no-one-click` for two steps.

To build and run the standalone Nuitka probe with a Python environment containing
Nuitka:

```powershell
./tools/nuitka-smoke.ps1 -Python python
```

## Testing

GitHub Actions runs on demand through **Run workflow** (`workflow_dispatch`)
and whenever a tag is pushed. It produces release packages for x64, x86, and
ARM64. After a successful tag build, the ZIPs are attached to a GitHub Release
with generated release notes; rerunning the job replaces assets with the same
names. Manual runs upload workflow artifacts without publishing a release.

```powershell
cargo test --workspace --exclude http-spike --target x86_64-pc-windows-msvc
cargo clippy --workspace --exclude http-spike --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo fmt --all -- --check

cargo build --target x86_64-pc-windows-msvc
python -m pip install cryptography
python tests/python/test_ctypes.py

./tools/abi-smoke.ps1 -Arch x64
```

HTTP tests need loopback networking. The native receive-timeout test takes about
30 seconds and runs separately:

```powershell
cargo test --test http --target x86_64-pc-windows-msvc native_read_timeout -- --ignored
```

Set `WINSPARKLE_DLL` to test a DLL other than the default debug build. The Python
suite uses its own local appcast server and the fictional Lumen Notes application.
No real installer is executed by the tests. ARM64 binaries and tests are
cross-linked on the x64 runner; native execution still requires Windows ARM64.

## Translations

The embedded translations are taken from
[WinSparkle's translation catalogs](https://github.com/vslavik/winsparkle/tree/master/translations).
Their original authors and contributors retain credit for that work. The public
C header also originates from WinSparkle.

The importer reads compiled gettext `.mo` catalogs, validates their structure and
encoding, applies overrides from `translations-overrides/`, and generates native
Windows `STRINGTABLE` resources. There is no gettext runtime in the DLL. The
current resources include 48 imported locales plus English; untranslated new
messages fall back to English, and Italian has complete overrides.

`resources/strings.toml` is the English source of truth. Files in
`translations-overrides/`, such as `it.toml`, map numeric resource IDs to local
translations that replace imported messages or translate new WinSparkle-NG
strings. Overrides apply to the base language and its regional variants during
import. Keep these files in version control so regenerating the resources
preserves those additions; do not edit the generated `.rc` files directly.

To regenerate resources, pass the translation directory of an external WinSparkle
checkout (for example, `../winsparkle/translations`):

```powershell
python -m pip install Babel
python tools/compile_catalogs.py --source ../winsparkle/translations
cargo xtask import-translations --source artifacts/catalogs
```

The importer writes coverage and unmapped messages to
`artifacts/translation-report.md`. Runtime language selection is described in the
[architecture guide](docs/architecture.md#resources-and-localization).

## Icon conversion

Convert a PNG or SVG to a multi-size ICO with Python 3.10+ and
[uv](https://docs.astral.sh/uv/). The script declares its Pillow and resvg-py
dependencies inline (PEP 723); `uv run` manages an isolated environment
automatically, without a tools-wide `pyproject.toml` or a separate Inkscape install.

```powershell
uv run tools/png_to_ico.py artwork.svg
uv run tools/png_to_ico.py artwork.png app.ico --sizes 256 128 48 32 16 --legacy-bmp
uv run tools/png_to_ico.py artwork.svg app.ico --crop 8
```

By default, the output uses the input filename with an `.ico` suffix and includes
256, 128, 64, 48, 32 and 16 pixel frames. SVGs are rasterized with resvg-py at the
largest requested width, preserving their aspect ratio and transparency; Pillow
creates the smaller frames. With `--crop`, the SVG render width includes twice
the crop margin so a square source still fills the largest requested frame.
Crop values refer to rasterized pixels for SVGs and source pixels for PNGs.
`--legacy-bmp` stores frames up to 64 pixels as raw BMP for older icon readers;
larger frames remain PNG.

## Design documentation

- [C API compatibility](docs/api-compatibility.md)
- [Architecture, update flow, and resources](docs/architecture.md)

Generated reports belong in the ignored `artifacts/` directory. Packaging writes
dependency audits, PE reports, and build metrics there; translation import writes
its coverage report there. These files describe a particular build or import and
are not maintained as project documentation.

## Credits and license

WinSparkle-NG is developed by **Gianluigi Tiesi <sherpya@gmail.com>**, with LLM
assistance, and is distributed under the [MIT License](LICENSE).

The original [WinSparkle](https://winsparkle.org/) project was created by
**Václav Slavík**. Its implementation served as the reference for this rewrite;
its translations and public header are reused with their MIT attribution
preserved. WinSparkle-NG is a separate project and is not an official WinSparkle
release.

Copyright (c) 2026 Gianluigi Tiesi <sherpya@gmail.com>.
WinSparkle-derived portions: Copyright (c) 2009–2026 Vaclav Slavik.
The shared license terms and copyright notices are included in [LICENSE](LICENSE).
