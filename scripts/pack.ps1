# 篝火 Bonfire 打包迁移（F1）：把安装包、迁移说明以及（可选）数据目录中的配置、备份仓库、日志打成一个 zip，
# 拷到新电脑上解压，运行安装包后把 data 放进安装目录即可继续使用。
# 用法（一般通过根目录的 pack.bat 调用）：
#   pack.bat                                交互：询问数据目录与要包含的内容，列出大小后确认
#   pack.bat --no-data --yes                只打包安装包与迁移说明
#   pack.bat --data "D:\Bonfire\data" --include config+repo --yes
# 始终排除：本机记住的密码（keys\，与这台电脑的 Windows 账户绑定，换电脑无效）、缓存（cache\）、写入中的临时文件。
param(
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

# 1. 版本号与安装包
$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$setup = Get-ChildItem (Join-Path $root "target\release\bundle\nsis\*.exe") -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { Write-Output "[错误] 没有找到安装包，请先运行 build.bat"; exit 2 }

# 2. 数据目录：命令行指定，或询问（默认不包含数据）
$items = @()
if (-not $NoData) {
  if ($Data -eq "" -and -not $Yes) {
    $default = Join-Path $env:LOCALAPPDATA "Bonfire\data"
    Write-Output "要一起打包篝火的数据吗？（配置、备份仓库、日志；默认不包含）"
    $Data = Read-Host "数据目录（直接回车跳过；安装版默认在 $default）"
  }
  if ($Data -ne "") {
    if (-not (Test-Path (Join-Path $Data "config.toml")) -and -not (Test-Path (Join-Path $Data "repo"))) {
      Write-Output "[错误] $Data 不是篝火的数据目录（没有 config.toml 或 repo）"; exit 2
    }
    $want = if ($Include -ne "") { $Include -split "[,+ ]" | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ } } else { @() }
    $cands = @(
      @{ key = "config"; name = "配置"; path = "config.toml"; default = $false },
      @{ key = "repo"; name = "备份仓库"; path = "repo"; default = $false },
      @{ key = "logs"; name = "日志"; path = "logs"; default = $false }
    )
    foreach ($c in $cands) {
      $p = Join-Path $Data $c.path
      if (-not (Test-Path $p)) { continue }
      $on = if ($Include -ne "") { $want -contains $c.key } else { Ask "包含$($c.name)（$($c.path)，$(Human (Size $p))）？" $c.default }
      if ($on) { $items += @{ name = $c.name; src = $p; rel = $c.path } }
    }
  }
}

# 3. 列出内容与大小，确认
$notes = Join-Path $root "docs\migration.md"
$total = $setup.Length + (Size $notes)
Write-Output ""
Write-Output "将打包："
Write-Output ("  安装包     {0}  {1}" -f $setup.Name, (Human $setup.Length))
Write-Output ("  迁移说明   migration.md  {0}" -f (Human (Size $notes)))
foreach ($i in $items) {
  $s = Size $i.src
  $total += $s
  Write-Output ("  {0}  data\{1}  {2}" -f $i.name, $i.rel, (Human $s))
}
Write-Output ("  合计（压缩前）{0}" -f (Human $total))
if ($items.Count -gt 0) { Write-Output "  不包含：本机记住的密码（keys\）、缓存（cache\）。加密的仓库在新电脑上需要输入密码。" }
if (-not (Ask "开始打包？" $true)) { Write-Output "已取消"; exit 1 }

# 4. 复制到临时目录后压缩
$stamp = Get-Date -Format "yyyyMMdd"
if ($Out -eq "") { $Out = Join-Path $root "dist\Bonfire_${version}_$stamp.zip" }
$stage = Join-Path $env:TEMP "bonfire-pack-$PID"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory $stage | Out-Null
try {
  Copy-Item $setup.FullName $stage
  Copy-Item $notes (Join-Path $stage "迁移说明.md")
  foreach ($i in $items) {
    $dst = Join-Path $stage ("data\" + $i.rel)
    New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null
    if ((Get-Item $i.src).PSIsContainer) {
      # 仓库中写入中途留下的临时文件与写入锁不需要。
      Copy-Item $i.src $dst -Recurse -Force
      Get-ChildItem $dst -Recurse -Force -Include "lock", "*.tmp" | Remove-Item -Force -ErrorAction SilentlyContinue
    } else {
      Copy-Item $i.src $dst -Force
    }
  }
  New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
  if (Test-Path $Out) { Remove-Item $Out -Force }
  Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $Out -CompressionLevel Optimal
} finally {
  Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Output ""
Write-Output ("完成：{0}（{1}）" -f $Out, (Human (Get-Item $Out).Length))
Write-Output "在新电脑上解压，运行安装包；包含数据时，安装后把 data 文件夹放进安装目录（默认 %LOCALAPPDATA%\Bonfire），详见迁移说明.md。"
exit 0
