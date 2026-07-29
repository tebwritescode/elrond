@echo off
setlocal

set "VSDEVCMD=%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat"
if not exist "%VSDEVCMD%" (
  echo Visual Studio Build Tools with the C++ workload are required. 1>&2
  exit /b 1
)

call "%VSDEVCMD%" -arch=x64 -host_arch=x64 >nul
if errorlevel 1 exit /b %errorlevel%

cargo fmt --all --check
if errorlevel 1 exit /b %errorlevel%

cargo clippy --workspace --all-targets --all-features -- -D warnings
if errorlevel 1 exit /b %errorlevel%

cargo test --workspace
