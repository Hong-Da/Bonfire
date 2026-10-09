@echo off
chcp 65001 >nul
setlocal EnableExtensions EnableDelayedExpansion
rem 篝火 Bonfire 一键运行（开发环境）：检查环境 → 安装依赖 → 构建并启动桌面应用。
rem 参数：--check 只检查环境；--reinstall 重新安装前端依赖；--cli 只构建命令行
cd /d "%~dp0"
if not exist logs mkdir logs
set "LOG=%~dp0logs\start.log"
echo ==== %date% %time% start.bat %* ====>>"%LOG%"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

set "MODE=run"
for %%A in (%*) do (
  if /i "%%~A"=="--check" set "MODE=check"
  if /i "%%~A"=="--cli" set "MODE=cli"
  if /i "%%~A"=="--reinstall" set "REINSTALL=1"
)

call :step 1 5 "检查 Rust 工具链"
where cargo >nul 2>&1
if errorlevel 1 (
  call :say "未找到 Rust（cargo）。"
  choice /c YN /m "是否用 winget 安装 rustup（Y/N）"
  if errorlevel 2 call :fail "需要 Rust 才能构建，请从 https://rustup.rs 安装后重试" & goto :end
  winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements || (call :fail "rustup 安装失败" & goto :end)
  rustup default stable >>"%LOG%" 2>&1
)
for /f "delims=" %%V in ('rustc --version') do call :say "  %%V"

call :step 2 5 "检查 MSVC 生成工具"
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VCOK="
if exist "%VSWHERE%" for /f "delims=" %%P in ('"%VSWHERE%" -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath') do set "VCOK=1"
if not defined VCOK (
  call :fail "未找到 Visual Studio C++ 生成工具，安装命令见 README「快速开始」"
  goto :end
)
call :say "  已安装"

if "%MODE%"=="cli" goto :buildcli

call :step 3 5 "检查 Node.js 与 pnpm"
where node >nul 2>&1 || (call :fail "未找到 Node.js，请从 https://nodejs.org 安装 20 以上版本" & goto :end)
where pnpm >nul 2>&1
if errorlevel 1 (
  choice /c YN /m "未找到 pnpm，是否用 npm 全局安装（Y/N）"
  if errorlevel 2 call :fail "需要 pnpm" & goto :end
  call npm install -g pnpm >>"%LOG%" 2>&1 || (call :fail "pnpm 安装失败" & goto :end)
)
for /f "delims=" %%V in ('node --version') do call :say "  Node.js %%V"
for /f "delims=" %%V in ('pnpm --version') do call :say "  pnpm %%V"

if "%MODE%"=="check" (
  call :say "环境检查通过。"
  goto :end
)

call :step 4 5 "安装前端依赖"
if defined REINSTALL if exist app\node_modules rmdir /s /q app\node_modules
call pnpm --dir app install --frozen-lockfile >>"%LOG%" 2>&1 || (call :fail "前端依赖安装失败，详见 logs\start.log" & goto :end)
call :say "  完成"

call :step 5 5 "构建并启动桌面应用（首次构建需要几分钟）"
call pnpm --dir app tauri dev
if errorlevel 1 call :fail "桌面应用启动失败" & goto :end
goto :end

:buildcli
call :step 3 3 "构建命令行 bonfire"
cargo build -p bonfire-cli || (call :fail "构建失败" & goto :end)
call :say "已构建 target\debug\bonfire.exe，可用 cli.bat 调用。"
goto :end

:step
call :say "[%~1/%~2] %~3……"
exit /b 0

:say
echo %~1
>>"%LOG%" echo %~1
exit /b 0

:fail
set "FAILED=1"
echo [错误] %~1
>>"%LOG%" echo [错误] %~1
exit /b 1

:end
if defined FAILED (
  if not defined BONFIRE_NO_PAUSE echo %cmdcmdline% | find /i "%~f0" >nul && pause
  exit /b 1
)
exit /b 0
