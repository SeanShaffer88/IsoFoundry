<#
.SYNOPSIS
  Builds IsoFoundry release assets from compiled binaries (D-044).

.DESCRIPTION
  Produces, in -OutDir:
    IsoFoundry-v<ver>-<arch>-Portable.zip       portable build (IsoFoundry.portable marker)
    IsoFoundry-v<ver>-<arch>-PortableApps.zip   PortableApps.com Format layout
    IsoFoundry-v<ver>-x64-Setup.exe             Inno Setup installer   (-Installers, x64 only)
    IsoFoundry-v<ver>-x64-Setup.msi             WiX v5 MSI             (-Installers, x64 only)
  and appends each file's hash to SHA256SUMS.txt.

  Run from the repository root after
    cargo build --workspace --release --locked --target <target>
#>
param(
  [Parameter(Mandatory)] [string] $Target,
  [Parameter(Mandatory)] [ValidateSet('x64', 'arm64')] [string] $Arch,
  [string] $OutDir = 'dist',
  [switch] $Installers
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$numeric = ($version -split '-')[0]                      # 0.1.0-dev -> 0.1.0 (MSI and PAF need numbers)
if ($numeric -notmatch '^\d+\.\d+\.\d+$') { throw "unexpected version '$version'" }
$rel = "target/$Target/release"
foreach ($f in 'isofoundry.exe', 'isofoundry-cli.exe', 'isofoundry-helper.exe', 'isofoundry-portable.exe') {
  if (-not (Test-Path "$rel/$f")) { throw "$rel/$f is missing; build the workspace first" }
}
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$work = Join-Path $OutDir "work-$Arch"
if (Test-Path $work) { Remove-Item -Recurse -Force $work }

function Add-Sum([string] $path) {
  $h = (Get-FileHash $path -Algorithm SHA256).Hash.ToLower()
  "$h  $(Split-Path $path -Leaf)" | Out-File -Append -Encoding ascii (Join-Path $OutDir 'SHA256SUMS.txt')
  Write-Host "built $(Split-Path $path -Leaf)"
}

function Copy-Program([string] $dest) {
  New-Item -ItemType Directory -Force $dest | Out-Null
  Copy-Item "$rel/isofoundry.exe", "$rel/isofoundry-helper.exe" $dest
  Copy-Item "$rel/isofoundry-cli.exe" (Join-Path $dest 'isofoundry.com')
}

# Staging folder shared by the installers.
$stage = Join-Path $work 'stage'
Copy-Program $stage
Copy-Item LICENSE, README.md, CHANGELOG.md, assets/icon.ico $stage

# 1. Portable zip.
$name = "IsoFoundry-v$version-$Arch-Portable"
$dir = Join-Path $work $name
Copy-Program $dir
Copy-Item LICENSE, README.md, CHANGELOG.md $dir
New-Item -ItemType File (Join-Path $dir 'IsoFoundry.portable') | Out-Null
$zip = Join-Path $OutDir "$name.zip"
Compress-Archive -Path "$dir/*" -DestinationPath $zip -Force
Add-Sum $zip

# 2. PortableApps.com Format layout.
$paf = Join-Path $work 'IsoFoundryPortable'
Copy-Program (Join-Path $paf 'App/IsoFoundry')
Copy-Item -Recurse packaging/paf/App/AppInfo (Join-Path $paf 'App/AppInfo')
Copy-Item packaging/paf/help.html $paf
Copy-Item "$rel/isofoundry-portable.exe" (Join-Path $paf 'IsoFoundryPortable.exe')
New-Item -ItemType Directory -Force (Join-Path $paf 'Other/Source'), (Join-Path $paf 'Data') | Out-Null
Copy-Item LICENSE, README.md (Join-Path $paf 'Other/Source')
$ini = Join-Path $paf 'App/AppInfo/appinfo.ini'
(Get-Content $ini -Raw).Replace('@PACKAGE_VERSION@', "$numeric.0").Replace('@DISPLAY_VERSION@', $version) |
  Set-Content -NoNewline -Encoding ascii $ini
$pafZip = Join-Path $OutDir "IsoFoundry-v$version-$Arch-PortableApps.zip"
Compress-Archive -Path $paf -DestinationPath $pafZip -Force
Add-Sum $pafZip

if (-not $Installers) { return }
if ($Arch -ne 'x64') { throw 'installers are built for x64 only (D-044)' }

# 3. Inno Setup installer (Setup.exe with the payload compiled in).
$iscc = @(
  (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
  "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
  "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 (ISCC.exe) not found' }
& $iscc /Q "/DAppVersion=$version" "/DSourceDir=$stage" "/DOutputDir=$OutDir" packaging/isofoundry.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
Add-Sum (Join-Path $OutDir "IsoFoundry-v$version-x64-Setup.exe")

# 4. WiX MSI.
$msi = Join-Path $OutDir "IsoFoundry-v$version-x64-Setup.msi"
wix build -arch x64 -d "Version=$numeric" -d "SourceDir=$stage" -o $msi packaging/isofoundry.wxs
if ($LASTEXITCODE -ne 0) { throw "wix build failed ($LASTEXITCODE)" }
Remove-Item -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($msi, 'wixpdb'))
Add-Sum $msi
