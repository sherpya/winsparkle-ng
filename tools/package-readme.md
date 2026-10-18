# WinSparkle-NG {{VERSION}} ({{ARCH}})

A standalone Windows updater DLL with a WinSparkle-compatible C API.
This package targets **{{ARCH}}** processes on Windows 10 and 11.

## Contents

- `WinSparkle.dll`: updater runtime, including the native UI and translations.
- `WinSparkle.lib`: MSVC import library for C/C++ applications.
- `winsparkle.h` and `winsparkle-version.h`: public C headers; keep both together.
- `winsparkle.py`: Python standard-library `ctypes` wrapper, also usable with Nuitka.
- `LICENSE`: MIT terms and copyright notices.
- `README.md`: this integration guide.

## Integration

Use the package matching your application's process architecture. Copy
`WinSparkle.dll` beside the application executable. C/C++ consumers include
`winsparkle.h` and link against `WinSparkle.lib`. Python consumers import
`WinSparkle` from `winsparkle.py` and pass the absolute DLL path to its constructor.

Configure application details, the appcast URL, and your trusted Ed25519 public
key before initialization. Keep callbacks alive until cleanup, and call
`win_sparkle_cleanup()` (or `WinSparkle.close()`) before unloading the DLL.
The Python wrapper retains the callback objects it registers.

One-click download-and-install is enabled by default. Use
`win_sparkle_set_install_after_download(0)` or `configure(..., one_click=False)`
for the traditional two-step flow. Themes default to the Windows application
preference and can be overridden through the C API or the Python wrapper.

Networking uses WinHTTP and Windows Schannel. No OpenSSL, libcurl, wxWidgets,
WebView2, or Visual C++ redistributable is required.

For complete API documentation, examples, source code, and limitations, see the
[project repository](https://github.com/sherpya/winsparkle-ng),
[C API reference](https://github.com/sherpya/winsparkle-ng/blob/master/docs/api-compatibility.md),
and [Python integration guide](https://github.com/sherpya/winsparkle-ng#python-and-nuitka).

## Credits

Developed by Gianluigi Tiesi <sherpya@gmail.com>, with LLM assistance.
[WinSparkle](https://winsparkle.org/), created by Václav Slavík, provided the
implementation reference, translations, and public header. MIT attribution is
preserved in `LICENSE`.
