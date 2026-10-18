# C API compatibility inventory

All entry points retain their undecorated `win_sparkle_*` names and `__cdecl`
calling convention (`extern "C"` in Rust). The original header remains usable;
the distributed header adds `win_sparkle_set_install_after_download` and
`win_sparkle_set_theme` without changing existing signatures or structures.
MSVC time_t defaults to signed 64 bits on both x86 and x64; the getter uses i64.
Pointers and `size_t` retain the architecture's native width. The callback
configuration struct uses `repr(C)` with original field order.

Configuration setters must be called before init, and copy strings/structures
before returning. The host owns callback function code and user_data through
cleanup. Callback setters, settings preferences, and the theme may be changed at runtime.
No pointer returned to the host needs freeing. NULL string inputs are handled
as empty strings. Non-NULL pointers must reference valid terminated storage.

Checks return immediately and coalesce while a check/download/prompt is active.
The UI and host callbacks run on the dedicated updater thread; config methods
can additionally run on the calling API thread. No global mutex is held across
host callbacks. Calls initiated from callbacks can set preferences or request
cleanup; unloading the DLL must wait until the callback has returned and the
updater thread has stopped. Normal external cleanup blocks until teardown.
Every C and Win32 callback boundary catches internal Rust unwinding. Foreign
exceptions and invalid host pointers are outside that boundary contract.

| Signature | Side effects / implemented behavior |
|---|---|
| `void __cdecl win_sparkle_init()` | Start one UI/controller thread; first-run consent and periodic scheduling. |
| `void __cdecl win_sparkle_set_install_after_download(int enabled)` | WinSparkle-NG extension; default 1 one-click, 0 opts into legacy two-step installation. Does not change automatic-install API. |
| `int __cdecl win_sparkle_set_theme(int theme)` | WinSparkle-NG extension: 0 auto (default), 1 light, 2 dark. Returns 1 on success, 0 for invalid input without changing the mode. May be changed at runtime; high contrast takes precedence. No host/global theme changes or persisted setting. |
| `void __cdecl win_sparkle_cleanup()` | Cancel I/O, join owned worker/UI threads, release resources and partial files; idempotent. |
| `void __cdecl win_sparkle_set_lang(const char *lang)` | Copy ISO locale; takes priority over LANGID. Full/base/English fallback. |
| `void __cdecl win_sparkle_set_langid(unsigned short lang)` | Set numeric Windows LANGID if no ISO locale was supplied. |
| `void __cdecl win_sparkle_set_appcast_url(const char *url)` | Copy HTTP(S) feed URL; otherwise host FeedURL/APPCAST resource. |
| `int __cdecl win_sparkle_set_dsa_pub_pem(const char *dsa_pub_pem)` | Return 0; retain a fail-closed unsupported DSA policy unless Ed25519 is configured. |
| `int __cdecl win_sparkle_set_eddsa_public_key(const char *pubkey)` | Copy Base64 trusted key; return 1 only for valid Ed25519 keys. Failed configuration remains fail-closed. |
| `void __cdecl win_sparkle_set_app_details(const wchar_t *company_name, const wchar_t *app_name, const wchar_t *app_version)` | Copy UTF-16 company, application and display version; otherwise host VERSIONINFO. |
| `void __cdecl win_sparkle_set_app_build_version(const wchar_t *build)` | Copy internal comparison version; does not replace display version. |
| `void __cdecl win_sparkle_set_http_header(const char *name, const char *value)` | Copy/replace case-insensitive header; reject CR/LF/NUL injection. |
| `void __cdecl win_sparkle_clear_http_headers()` | Remove copied custom headers. |
| `void __cdecl win_sparkle_set_registry_path(const char *path)` | Copy HKCU/HKLM-relative settings path. |
| `void __cdecl win_sparkle_set_config_methods(win_sparkle_config_methods_t *config_methods)` | Copy all three callbacks and opaque data; NULL restores registry. Incomplete sets are ignored. |
| `void __cdecl win_sparkle_set_automatic_check_for_updates(int state)` | Write CheckForUpdates as REG_SZ or custom storage. |
| `int __cdecl win_sparkle_get_automatic_check_for_updates()` | Read CheckForUpdates; default 0. |
| `void __cdecl win_sparkle_set_update_check_interval(int interval)` | Write UpdateInterval; clamp to at least 3600 seconds. |
| `int __cdecl win_sparkle_get_update_check_interval()` | Read UpdateInterval; default 86400 seconds; clamp malformed lower values. |
| `time_t __cdecl win_sparkle_get_last_check_time()` | Read signed 64-bit time_t LastCheckTime; default -1. |
| `void __cdecl win_sparkle_set_error_callback(win_sparkle_error_callback_t callback)` | Store nullable error callback. Invoked without global locks; host owns its lifetime. |
| `void __cdecl win_sparkle_set_can_shutdown_callback(win_sparkle_can_shutdown_callback_t callback)` | Store nullable can_shutdown callback. Invoked without global locks; host owns its lifetime. 0 retains the downloaded file and offers installation retry. |
| `void __cdecl win_sparkle_set_shutdown_request_callback(win_sparkle_shutdown_request_callback_t)` | Store nullable shutdown_request callback. Invoked without global locks; host owns its lifetime. Called only after successful installer launch. |
| `void __cdecl win_sparkle_set_did_find_update_callback(win_sparkle_did_find_update_callback_t callback)` | Store nullable did_find_update callback. Invoked without global locks; host owns its lifetime. |
| `void __cdecl win_sparkle_set_did_not_find_update_callback(win_sparkle_did_not_find_update_callback_t callback)` | Store nullable did_not_find_update callback. Invoked without global locks; host owns its lifetime. |
| `void __cdecl win_sparkle_set_update_cancelled_callback(win_sparkle_update_cancelled_callback_t callback)` | Store nullable update_cancelled callback. Invoked without global locks; host owns its lifetime. |
| `void __cdecl win_sparkle_set_update_skipped_callback(win_sparkle_update_skipped_callback_t callback)` | Store nullable update_skipped callback. Invoked without global locks; host owns its lifetime. |
| `void __cdecl win_sparkle_set_update_postponed_callback(win_sparkle_update_postponed_callback_t callback)` | Store nullable update_postponed callback. Invoked without global locks; host owns its lifetime. |
| `void __cdecl win_sparkle_set_update_dismissed_callback(win_sparkle_update_dismissed_callback_t callback)` | Store nullable update_dismissed callback. Invoked without global locks; host owns its lifetime. |
| `void __cdecl win_sparkle_set_user_run_installer_callback(win_sparkle_user_run_installer_callback_t callback)` | Store nullable user_run_installer callback. Invoked without global locks; host owns its lifetime. 0 delegates to ShellExecuteExW, 1 succeeds, -1/other fails. |
| `void __cdecl win_sparkle_check_update_with_ui()` | Queue manual check; visible progress and ignore skipped version. |
| `void __cdecl win_sparkle_check_update_with_ui_and_install()` | Queue manual check; skip initial update prompt and install after download/verification. |
| `void __cdecl win_sparkle_check_update_without_ui()` | Queue background check; respect skipped version, show update prompt when applicable. |

Intentional differences: DSA unsupported; invalid key configuration never falls
back to unsigned; critical updates cannot be skipped; release notes use safe text
and external links; malformed length/XML/URL inputs are rejected. Numeric version
segments are compared without the original atoi overflow. Original HKCU then HKLM
settings fallback and REG_SZ format are preserved.

Registry values: CheckForUpdates, DidRunOnce, UpdateInterval, LastCheckTime,
SkipThisVersion and UpdateTempDir. Default path:
HKCU\Software\<company>\<application>\WinSparkle (company omitted if empty).
Only a validated WinSparkle-NG temp directory can be removed via UpdateTempDir.

`tools/abi-smoke.ps1` compiles an actual C executable against the distributed
header and runs it with the DLL. Pass `-HeaderDirectory <path>` to test an external
WinSparkle header. `tools/package.ps1` compares every header export to
the PE export table and audits imports/Cargo dependencies. x86 and x64 executables
were run; the ARM64 DLL and C consumer are cross-linked, with native execution
still requiring Windows ARM64.
