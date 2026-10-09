@echo off
chcp 65001 >nul
setlocal EnableExtensions
rem 篝火 Bonfire 构建安装包：命令行 → 内置规则库快照 → NSIS 安装包（含桌面应用与命令行 bonfire.exe）。
rem 参数：--offline 不下载规则库（使用已有快照；没有快照时安装包不含规则库）
cd /d "%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
set "OFFLINE="
for %%A in (%*) do if /i "%%~A"=="--offline" set "OFFLINE=1"

echo [1/4] 检查环境……
call "%~dp0start.bat" --check >nul || (echo [错误] 环境检查未通过，请先运行 start.bat --check 查看详情 & goto :fail)

echo [2/4] 构建命令行 bonfire（release）……
cargo build --release -p bonfire-cli || goto :fail
if not exist app\src-tauri\binaries mkdir app\src-tauri\binaries
copy /y target\release\bonfire.exe app\src-tauri\binaries\bonfire-x86_64-pc-windows-msvc.exe >nul || goto :fail

echo [3/4] 准备内置规则库快照……
if not exist app\src-tauri\resources mkdir app\src-tauri\resources
if defined OFFLINE (
  if not exist app\src-tauri\resources\manifest.json.zst echo [提示] 没有规则库快照，安装包首次启动后需要联网更新规则库
) else (
  set "BONFIRE_HOME=%TEMP%\bonfire-build-home"
  target\release\bonfire.exe rules update --bundle app\src-tauri\resources\manifest.json.zst || goto :fail
)
if not exist app\src-tauri\resources\manifest.json.zst type nul > app\src-tauri\resources\manifest.json.zst

echo [4/4] 构建桌面应用与安装包……
call pnpm --dir app install --frozen-lockfile || goto :fail
call pnpm --dir app tauri build --config src-tauri/tauri.bundle.json || goto :fail

echo.
echo 完成：
dir /b target\release\bundle\nsis\*.exe
exit /b 0

:fail
echo [错误] 构建失败
if not defined BONFIRE_NO_PAUSE echo %cmdcmdline% | find /i "%~f0" >nul && pause
exit /b 1
