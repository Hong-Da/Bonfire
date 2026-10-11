@echo off
chcp 65001 >nul
setlocal EnableExtensions
rem 篝火 Bonfire 打包迁移：安装包 + 迁移说明 +（可选）数据目录中的配置、备份仓库、日志 → dist\Bonfire_<版本>_<日期>.zip
rem 参数：--no-data 不包含数据；--data <数据目录>；--include config+repo+logs；--yes 不询问（按参数或默认）；--out <zip 路径>
rem 先运行 build.bat 生成安装包。详见 docs\migration.md。
cd /d "%~dp0"
rem shift 之后 %~dp0 会变，先记下脚本所在目录。
set "HERE=%~dp0"
set "ARGS="
:loop
if "%~1"=="" goto run
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
