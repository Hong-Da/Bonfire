# 篝火 Bonfire 打包（F1）：生成发布包，文件名带版本号与运行环境（如 win-x64）。先运行 build.bat。
#
#   类型（--type，默认免安装包）：
#     portable  免安装包  dist\Bonfire_<版本>_<环境>_portable.zip     解压即用，数据在同目录的 data 中；可选带上数据
#     setup     安装包    dist\Bonfire_<版本>_<环境>_setup.exe        NSIS 安装包（改为统一的文件名）
#     migrate   迁移包    dist\Bonfire_<版本>_<环境>_migrate_<日期>.zip  安装包 + 迁移说明 + 可选的数据
#     all       免安装包与安装包（发布时用）
#   用法（一般通过根目录的 pack.bat 调用）：
#     pack.bat                                     交互：选择类型（回车为免安装包），需要时询问数据
#     pack.bat --type portable --no-data --yes     只打免安装包，不带数据
#     pack.bat --type migrate --data "D:\Bonfire\data" --include config+repo --yes
#   运行环境：从 Bonfire.exe 的文件头读取 CPU 架构（x64 / x86 / arm64），系统固定为 win。
#   带数据时始终排除：本机记住的密码与同步令牌（keys\，与这台电脑的 Windows 账户绑定，换电脑无效）、缓存（cache\）、
#   窗口数据（webview\）、写入中的临时文件与写入锁。
param(
  [string]$Type = "",
  [string]$Data = "",
  [string]$Include = "",
  [switch]$NoData,
  [switch]$Yes,
  [string]$Out = ""
)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

function Size($path) {
  if (-not (Test-Path $path)) { return 0 }
  $s = (Get-ChildItem $path -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
  if ($null -eq $s) { 0 } else { $s }
}
function Human($n) {
  if ($n -ge 1GB) { "{0:N1} GB" -f ($n / 1GB) } elseif ($n -ge 1MB) { "{0:N1} MB" -f ($n / 1MB) } elseif ($n -ge 1KB) { "{0:N1} KB" -f ($n / 1KB) } else { "$n B" }
}
function Ask($q, $default) {
  if ($Yes) { return $default }
  $d = if ($default) { "Y/n" } else { "y/N" }
  $a = Read-Host "$q [$d]"
  if ($a -eq "") { return $default }
  return $a -match "^(y|yes|是)$"
}
# PE 文件头：Machine（0x8664 x64、0x14c x86、0xaa64 arm64）与 Subsystem（2 图形界面、3 控制台）。
function PeInfo($path) {
  $b = [IO.File]::ReadAllBytes($path)
  $pe = [BitConverter]::ToInt32($b, 0x3C)
  $machine = [BitConverter]::ToUInt16($b, $pe + 4)
  $arch = switch ($machine) { 0x8664 { "x64" } 0x14c { "x86" } 0xaa64 { "arm64" } default { "unknown" } }
  @{ arch = $arch; subsystem = [BitConverter]::ToUInt16($b, $pe + 24 + 68) }
}

# 1. 类型
$types = @{ "1" = "portable"; "2" = "setup"; "3" = "migrate"; "4" = "all" }
if ($Type -eq "") {
  if ($Yes) { $Type = "portable" } else {
    Write-Output "要打哪种包？"
    Write-Output "  1  免安装包：解压即用，数据保存在同目录（默认）"
    Write-Output "  2  安装包：NSIS 安装程序"
    Write-Output "  3  迁移包：安装包 + 迁移说明 + 可选的数据，换电脑用"
    Write-Output "  4  全部：免安装包与安装包（发布时用）"
    $a = Read-Host "选择 [1]"
    $Type = if ($a -eq "") { "portable" } elseif ($types.ContainsKey($a)) { $types[$a] } else { $a }
  }
}
$Type = $Type.ToLower()
if (@("portable", "setup", "migrate", "all") -notcontains $Type) { Write-Output "[错误] 未知的类型 $Type（可选 portable、setup、migrate、all）"; exit 2 }

# 2. 构建产物、版本号与运行环境
$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$desktop = Join-Path $root "target\release\Bonfire.exe"
$cli = Join-Path $root "app\src-tauri\binaries\bonfire-x86_64-pc-windows-msvc.exe"
$rules = Join-Path $root "app\src-tauri\resources\manifest.json.zst"
$setup = Get-ChildItem (Join-Path $root "target\release\bundle\nsis\Bonfire_${version}_*-setup.exe") -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
foreach ($f in @($desktop, $cli)) { if (-not (Test-Path $f)) { Write-Output "[错误] 没有找到 $f，请先运行 build.bat"; exit 2 } }
if (($Type -ne "portable") -and -not $setup) { Write-Output "[错误] 没有找到 $version 版的安装包，请先运行 build.bat"; exit 2 }
$pe = PeInfo $desktop
if ($pe.subsystem -ne 2) { Write-Output "[错误] $desktop 不是桌面程序（可能被命令行覆盖），请重新运行 build.bat"; exit 2 }
if ($pe.arch -eq "unknown") { Write-Output "[错误] 无法识别 Bonfire.exe 的 CPU 架构"; exit 2 }
$envName = "win-" + $pe.arch
$base = "Bonfire_${version}_$envName"
$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force $dist | Out-Null

# 3. 数据（免安装包与迁移包可选；默认不包含）
$items = @()
if (($Type -eq "portable" -or $Type -eq "migrate") -and -not $NoData) {
  if ($Data -eq "" -and -not $Yes) {
    $default = Join-Path $root "data"
    Write-Output ""
    Write-Output "要一起打包篝火的数据吗？（配置、备份仓库、日志；默认不包含）"
    $Data = Read-Host "数据目录（直接回车跳过；例如 $default 或 <安装目录>\data）"
  }
  if ($Data -ne "") {
    if (-not (Test-Path (Join-Path $Data "config.toml")) -and -not (Test-Path (Join-Path $Data "repo"))) {
      Write-Output "[错误] $Data 不是篝火的数据目录（没有 config.toml 或 repo）"; exit 2
    }
    $want = if ($Include -ne "") { $Include -split "[,+ ]" | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ } } else { @() }
    $cands = @(
      @{ key = "config"; name = "配置"; path = "config.toml" },
      @{ key = "config"; name = "自定义规则"; path = "custom-rules.yaml" },
      @{ key = "repo"; name = "备份仓库"; path = "repo" },
      @{ key = "logs"; name = "日志"; path = "logs" }
    )
    foreach ($c in $cands) {
      $p = Join-Path $Data $c.path
      if (-not (Test-Path $p)) { continue }
      $on = if ($Include -ne "") { $want -contains $c.key } else { Ask "包含$($c.name)（$($c.path)，$(Human (Size $p))）？" $false }
      if ($on) { $items += @{ name = $c.name; src = $p; rel = $c.path } }
    }
  }
}

function Stage() {
  $s = Join-Path $env:TEMP "bonfire-pack-$PID-$([guid]::NewGuid().ToString('N').Substring(0, 6))"
  New-Item -ItemType Directory $s | Out-Null
  $s
}
function CopyData($dataDir) {
  foreach ($i in $items) {
    $dst = Join-Path $dataDir $i.rel
    New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null
    if ((Get-Item $i.src).PSIsContainer) {
      Copy-Item $i.src $dst -Recurse -Force
      # 写入中途留下的临时文件与写入锁不需要。
      Get-ChildItem $dst -Recurse -Force -Include "lock", "*.tmp" | Remove-Item -Force -ErrorAction SilentlyContinue
    } else {
      Copy-Item $i.src $dst -Force
    }
  }
}
function Zip($stage, $zip) {
  if (Test-Path $zip) { Remove-Item $zip -Force }
  Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip -CompressionLevel Optimal
}

$done = @()

# 4a. 免安装包：解压后是一个文件夹，结构与安装目录相同（Bonfire.exe、cli\bonfire.exe、规则库），另加 portable 标记与说明。
if ($Type -eq "portable" -or $Type -eq "all") {
  $zip = if ($Out -ne "" -and $Type -eq "portable") { $Out } else { Join-Path $dist "${base}_portable.zip" }
  Write-Output ""
  Write-Output ("免安装包：{0}" -f (Split-Path $zip -Leaf))
  Write-Output ("  桌面程序 Bonfire.exe  {0}；命令行 cli\bonfire.exe  {1}；规则库 {2}" -f (Human (Get-Item $desktop).Length), (Human (Get-Item $cli).Length), (Human (Size $rules)))
  foreach ($i in $items) { Write-Output ("  {0}  data\{1}  {2}" -f $i.name, $i.rel, (Human (Size $i.src))) }
  if (-not (Ask "开始打包？" $true)) { Write-Output "已取消"; exit 1 }
  $stage = Stage
  try {
    $dir = Join-Path $stage $base
    New-Item -ItemType Directory -Force (Join-Path $dir "cli") | Out-Null
    Copy-Item $desktop (Join-Path $dir "Bonfire.exe")
    Copy-Item $cli (Join-Path $dir "cli\bonfire.exe")
    if ((Size $rules) -gt 0) { Copy-Item $rules (Join-Path $dir "manifest.json.zst") }
    Set-Content (Join-Path $dir "portable") "篝火免安装版：数据保存在本目录的 data 文件夹中。" -Encoding utf8
    $readme = @"
篝火 Bonfire $version 免安装版（$envName）

使用
  双击 Bonfire.exe 运行。第一次运行会进入首次运行向导。
  命令行：cli\bonfire.exe（例如 cli\bonfire.exe status）。

数据
  设置、备份仓库、日志都保存在本目录的 data 文件夹中，不写注册表、不写用户目录。
  换电脑或放进 U 盘：把整个文件夹拷过去即可（本机记住的密码与同步令牌需要重新输入）。
  卸载：退出篝火（托盘菜单“退出”）后删除整个文件夹；要保留备份请先拷出 data\repo。

运行环境
  Windows 10 / 11（$envName）。需要系统自带的 Microsoft Edge WebView2 运行库；
  极少数旧系统没有时，请先从微软官网安装“WebView2 Runtime”。
"@
    Set-Content (Join-Path $dir "使用说明.txt") $readme -Encoding utf8
    if ($items.Count -gt 0) { CopyData (Join-Path $dir "data") }
    Zip $stage $zip
  } finally { Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue }
  $done += $zip
}

# 4b. 安装包：复制为统一的文件名。
if ($Type -eq "setup" -or $Type -eq "all") {
  $exe = if ($Out -ne "" -and $Type -eq "setup") { $Out } else { Join-Path $dist "${base}_setup.exe" }
  Copy-Item $setup.FullName $exe -Force
  $done += $exe
}

# 4c. 迁移包：安装包 + 迁移说明 + 可选的数据。
if ($Type -eq "migrate") {
  $stamp = Get-Date -Format "yyyyMMdd"
  $zip = if ($Out -ne "") { $Out } else { Join-Path $dist "${base}_migrate_$stamp.zip" }
  $notes = Join-Path $root "docs\migration.md"
  $total = $setup.Length + (Size $notes)
  Write-Output ""
  Write-Output ("迁移包：{0}" -f (Split-Path $zip -Leaf))
  Write-Output ("  安装包     {0}_setup.exe  {1}" -f $base, (Human $setup.Length))
  Write-Output ("  迁移说明   迁移说明.md  {0}" -f (Human (Size $notes)))
  foreach ($i in $items) { $s = Size $i.src; $total += $s; Write-Output ("  {0}  data\{1}  {2}" -f $i.name, $i.rel, (Human $s)) }
  Write-Output ("  合计（压缩前）{0}" -f (Human $total))
  if ($items.Count -gt 0) { Write-Output "  不包含：本机记住的密码与同步令牌（keys\）、缓存（cache\）。加密的仓库在新电脑上需要输入密码。" }
  if (-not (Ask "开始打包？" $true)) { Write-Output "已取消"; exit 1 }
  $stage = Stage
  try {
    Copy-Item $setup.FullName (Join-Path $stage "${base}_setup.exe")
    Copy-Item $notes (Join-Path $stage "迁移说明.md")
    if ($items.Count -gt 0) { CopyData (Join-Path $stage "data") }
    Zip $stage $zip
  } finally { Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue }
  $done += $zip
}

Write-Output ""
Write-Output "完成："
foreach ($f in $done) { Write-Output ("  {0}（{1}）" -f $f, (Human (Get-Item $f).Length)) }
exit 0
