@echo off
chcp 65001 >nul
setlocal
rem 调用命令行 bonfire：先增量构建，再把全部参数原样传入。不切换当前目录，相对路径以调用位置为准。
set "ROOT=%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cargo build -q -p bonfire-cli --manifest-path "%ROOT%Cargo.toml"
if errorlevel 1 exit /b 1
"%ROOT%target\debug\bonfire.exe" %*
exit /b %ERRORLEVEL%
