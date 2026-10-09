@echo off
chcp 65001 >nul
setlocal EnableExtensions EnableDelayedExpansion
rem 篝火 Bonfire 一键测试。
rem 无参数：全部测试；--unit 单元 / --cli CLI / --docs 文档 / --ui 前端；--bless 重新生成文档中的生成内容
cd /d "%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if not exist tests\output mkdir tests\output
set "REPORT=tests\output\report.txt"
echo 篝火 Bonfire 测试报告 %date% %time%>"%REPORT%"

set "SEL="
for %%A in (%*) do (
  if /i "%%~A"=="--unit" set "SEL=!SEL! unit"
  if /i "%%~A"=="--cli" set "SEL=!SEL! cli"
  if /i "%%~A"=="--docs" set "SEL=!SEL! docs"
  if /i "%%~A"=="--ui" set "SEL=!SEL! ui"
  if /i "%%~A"=="--bless" set "BONFIRE_BLESS=1" & set "SEL=!SEL! docs"
)
if not defined SEL set "SEL=unit cli docs ui"

rem CLI 测试执行真实二进制，先构建。
echo %SEL% | find "cli" >nul && (cargo build -q -p bonfire-cli || goto :buildfail)

set "FAILS=0"
for %%S in (%SEL%) do call :run %%S
echo.
echo ==== 汇总 ====
type "%REPORT%" | find "："
if %FAILS% gtr 0 (
  echo 有 %FAILS% 类测试失败，详见 tests\output\*.log
  if not defined BONFIRE_NO_PAUSE echo %cmdcmdline% | find /i "%~f0" >nul && pause
  exit /b 1
)
echo 全部通过。
exit /b 0

:run
set "CAT=%~1"
set "OUT=tests\output\%CAT%.log"
echo.
echo ==== %CAT% ====
if "%CAT%"=="unit" cargo test --workspace --exclude bonfire-tests >"%OUT%" 2>&1
if "%CAT%"=="cli" cargo test -p bonfire-tests --test cli --test crash >"%OUT%" 2>&1
if "%CAT%"=="docs" cargo test -p bonfire-tests --test docs >"%OUT%" 2>&1
if "%CAT%"=="ui" call pnpm --dir app test >"%OUT%" 2>&1
set "RC=%ERRORLEVEL%"
type "%OUT%" | findstr /r /c:"test result" /c:"Tests " /c:"FAILED" /c:"panicked"
if "%RC%"=="0" (
  echo %CAT%：通过>>"%REPORT%"
) else (
  set /a FAILS+=1
  echo %CAT%：失败（退出码 %RC%）>>"%REPORT%"
)
exit /b 0

:buildfail
echo [错误] 构建 bonfire 失败
exit /b 1
