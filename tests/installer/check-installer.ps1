# 安装包验收（M3-6）：静默安装到临时目录 → 检查文件、命令行与规则库 → 写入数据标记 → 覆盖安装（升级）
# → 卸载，检查程序已删除、数据仍保留、卸载登记与开始菜单快捷方式已清除 → 再次安装后用 /DELETEDATA 卸载，检查数据已删除。
# 用法：先运行 build.bat 生成安装包，再运行
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\installer\check-installer.ps1
# 不启动桌面程序（标识与开发版相同，会唤起正在运行的篝火）；所有数据都在临时目录，用完即删除。
# 本机已安装篝火时不运行（卸载登记同名，测试的卸载会清掉它）。
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Bonfire"
if (Test-Path $key) {
  Write-Output "本机已安装篝火（$((Get-ItemProperty $key).InstallLocation)），为避免影响它，不运行安装测试。"
  exit 2
}
$setup = Get-ChildItem "$root\target\release\bundle\nsis\*.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { Write-Output "没有找到安装包，请先运行 build.bat"; exit 2 }
$dir = Join-Path $env:TEMP "bonfire-install-test\Bonfire"
$report = [ordered]@{}
$report.installer = "$($setup.Name)  $([math]::Round($setup.Length / 1MB, 1)) MB"
if (Test-Path (Split-Path $dir)) { Remove-Item (Split-Path $dir) -Recurse -Force }

function Install() {
  $p = Start-Process $setup.FullName -ArgumentList "/S", "/D=$dir" -Wait -PassThru
  return $p.ExitCode
}

function Shortcuts() {
  (Get-ChildItem "$env:APPDATA\Microsoft\Windows\Start Menu\Programs" -Recurse -Filter "*.lnk" |
    Where-Object { $_.Name -like "*Bonfire*" -or $_.Name -like "*篝火*" } | ForEach-Object Name) -join ", "
}

# 读 PE 头的子系统：2 = 图形界面，3 = 控制台。
function Subsystem($path) {
  $b = [IO.File]::ReadAllBytes($path)
  $pe = [BitConverter]::ToInt32($b, 0x3C)
  return [BitConverter]::ToUInt16($b, $pe + 24 + 68)
}

# 1. 安装
$report.installExit = Install
$report.files = (Get-ChildItem $dir -Recurse -File | ForEach-Object { $_.FullName.Substring($dir.Length + 1) }) -join "; "
$report.uninstallKey = Test-Path $key
if ($report.uninstallKey) { $report.displayVersion = (Get-ItemProperty $key).DisplayVersion }
$report.startMenu = Shortcuts

# 2. 命令行（在 cli 子目录；数据与桌面程序共用安装目录下的 data）
$env:BONFIRE_OFFLINE = "1"
Remove-Item Env:BONFIRE_HOME -ErrorAction SilentlyContinue
Remove-Item Env:BONFIRE_CONFIG -ErrorAction SilentlyContinue
$cli = Join-Path $dir "cli\bonfire.exe"
$report.cliVersion = (& $cli --version) -join " "
$report.rules = (& $cli rules info 2>&1) -join " "
$null = & $cli status 2>&1
$report.dataDir = (Get-ChildItem (Join-Path $dir "data") -ErrorAction SilentlyContinue | ForEach-Object Name) -join ", "
$report.desktopSubsystem = Subsystem (Join-Path $dir "Bonfire.exe")
$report.cliSubsystem = Subsystem $cli

# 3. 数据标记（模拟用户的仓库）
Set-Content (Join-Path $dir "data\repo\keep-me.txt") "user data" -Encoding utf8

# 4. 覆盖安装（升级）
$report.upgradeExit = Install
$report.dataAfterUpgrade = Test-Path (Join-Path $dir "data\repo\keep-me.txt")

# 5. 卸载
Start-Process (Join-Path $dir "uninstall.exe") -ArgumentList "/S" -Wait
for ($i = 0; $i -lt 60 -and (Test-Path (Join-Path $dir "Bonfire.exe")); $i++) { Start-Sleep -Milliseconds 500 }
$report.exeAfterUninstall = Test-Path (Join-Path $dir "Bonfire.exe")
$report.cliAfterUninstall = Test-Path $cli
$report.dataAfterUninstall = Test-Path (Join-Path $dir "data\repo\keep-me.txt")
$report.uninstallKeyAfter = Test-Path $key
$report.startMenuAfter = Shortcuts

# 6. 再次安装后卸载并删除数据（静默卸载默认保留数据；/DELETEDATA 删除，界面卸载时询问）
$report.reinstallExit = Install
Start-Process (Join-Path $dir "uninstall.exe") -ArgumentList "/S", "/DELETEDATA" -Wait
for ($i = 0; $i -lt 60 -and (Test-Path (Join-Path $dir "data")); $i++) { Start-Sleep -Milliseconds 500 }
$report.dataAfterDeleteData = Test-Path (Join-Path $dir "data")
$report.dirAfterDeleteData = Test-Path $dir

Remove-Item (Split-Path $dir) -Recurse -Force -ErrorAction SilentlyContinue
$report.passed = $report.installExit -eq 0 -and $report.upgradeExit -eq 0 -and
  $report.desktopSubsystem -eq 2 -and $report.cliSubsystem -eq 3 -and
  $report.rules -match "规则库：[1-9]\d* 个游戏|rules: [1-9]" -and $report.dataDir -match "repo" -and
  $report.dataAfterUpgrade -and -not $report.exeAfterUninstall -and -not $report.cliAfterUninstall -and
  $report.dataAfterUninstall -and -not $report.uninstallKeyAfter -and -not $report.startMenuAfter -and
  $report.reinstallExit -eq 0 -and -not $report.dataAfterDeleteData -and -not $report.dirAfterDeleteData
$report | ConvertTo-Json
if (-not $report.passed) { exit 1 }
