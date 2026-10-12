@echo off
chcp 65001 >nul
setlocal EnableExtensions
rem 篝火 Bonfire 打包：文件名带版本号与运行环境，如 dist\Bonfire_<版本>_win-x64_portable.zip
rem 参数：--type portable（免安装包，默认）/ setup（安装包）/ migrate（迁移包：安装包 + 迁移说明 + 数据）/ all（免安装包与安装包）
rem       --no-data 不包含数据；--data <数据目录>；--include config+repo+logs；--yes 不询问（按参数或默认）；--out <输出路径>
rem 先运行 build.bat。详见 docs\migration.md。
cd /d "%~dp0"
rem shift 之后 %~dp0 会变，先记下脚本所在目录。
set "HERE=%~dp0"
set "ARGS="
:loop
if "%~1"=="" goto run
if /i "%~1"=="--type" (set "ARGS=%ARGS% -Type %~2" & shift & shift & goto loop)
if /i "%~1"=="--no-data" (set "ARGS=%ARGS% -NoData" & shift & goto loop)
if /i "%~1"=="--yes" (set "ARGS=%ARGS% -Yes" & shift & goto loop)
if /i "%~1"=="--data" (set "ARGS=%ARGS% -Data "%~2"" & shift & shift & goto loop)
if /i "%~1"=="--include" (set "ARGS=%ARGS% -Include %~2" & shift & shift & goto loop)
if /i "%~1"=="--out" (set "ARGS=%ARGS% -Out "%~2"" & shift & shift & goto loop)
echo [错误] 未知参数 %~1
exit /b 2
:run
powershell -NoProfile -ExecutionPolicy Bypass -File "%HERE%scripts\pack.ps1" %ARGS%
set "CODE=%ERRORLEVEL%"
if not defined BONFIRE_NO_PAUSE echo %cmdcmdline% | find /i "%HERE%pack.bat" >nul && pause
exit /b %CODE%
