@echo off
setlocal
rem =====================================================================
rem  TREMOR - local Windows build + launch
rem  Requires a Rust toolchain (install from https://rustup.rs).
rem  Works with MSVC or GNU toolchains; no other dependencies.
rem  Usage:  launch.bat            build (if stale) + run
rem          launch.bat --sim      run with the offline simulation feed
rem          launch.bat rebuild    force a clean release rebuild
rem =====================================================================
where cargo >nul 2>nul
if errorlevel 1 (
    echo [tremor] cargo not found in PATH.
    echo [tremor] Install the Rust toolchain from https://rustup.rs and retry.
    pause
    exit /b 1
)
if /i "%1"=="rebuild" (
    echo [tremor] forcing release rebuild ...
    cargo build --release --target-dir target
    shift
) else (
    echo [tremor] building release exe (no-op if up to date) ...
    cargo build --release
)
if errorlevel 1 (
    echo [tremor] BUILD FAILED - see cargo output above.
    pause
    exit /b 1
)
echo [tremor] launching target\release\tremor.exe %1 %2 %3 %4
target\release\tremor.exe %1 %2 %3 %4
if errorlevel 1 (
    echo [tremor] exited with error code %errorlevel%
    pause
)
endlocal
