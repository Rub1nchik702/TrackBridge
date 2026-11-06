param([string]$OutputDirectory = (Join-Path $PSScriptRoot 'dist'))
$ErrorActionPreference = 'Stop'
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (!(Test-Path -LiteralPath $compiler)) { throw '.NET Framework C# compiler is required.' }
$files = @('Core.cs','Localizer.cs','SigningKey.cs','Resolver.cs','Updates.cs','ChatCommands.cs','Listener.cs','Program.cs','Dashboard.cs','Brand.cs','Tests.cs','AssemblyInfo.cs') | ForEach-Object { Join-Path $PSScriptRoot ('src\' + $_) }
$versionText = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'src\Updates.cs'))
$version = [regex]::Match($versionText, 'Current\s*=\s*"([0-9.]+(?:-beta\.[0-9]+)?)"').Groups[1].Value
if (!$version) { throw 'Version is missing.' }
$stage = Join-Path $OutputDirectory 'TrackBridge'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$icon = Join-Path $PSScriptRoot 'assets\TrackBridge.ico'
$exe = Join-Path $stage 'TrackBridge.exe'
& $compiler /nologo /target:winexe /optimize+ /debug- /utf8output /win32icon:$icon /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Net.Http.dll /r:System.Web.dll /r:System.Web.Extensions.dll /r:System.Security.dll /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll /out:$exe $files
if ($LASTEXITCODE -ne 0) { throw 'Compilation failed.' }
$documents = @('README.md','README.en.md','LICENSE.txt','THIRD-PARTY-NOTICES.txt','CHANGELOG.md','TRADEMARKS.md')
foreach ($file in $documents) { Copy-Item -LiteralPath (Join-Path $PSScriptRoot $file) -Destination (Join-Path $stage $file) -Force }
$readme = [Net.WebUtility]::HtmlEncode([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'README.md')))
[IO.File]::WriteAllText((Join-Path $stage 'README.html'),('<!doctype html><meta charset="utf-8"><title>TrackBridge</title><style>body{background:#161920;color:#e6eaf1;font:16px/1.6 system-ui;margin:40px}pre{white-space:pre-wrap}</style><pre>' + $readme + '</pre>'))
$archiveName = 'TrackBridge-' + $version + '-win.zip'
$archive = Join-Path $OutputDirectory $archiveName
$packageFiles = @('TrackBridge.exe','README.md','README.html','LICENSE.txt','THIRD-PARTY-NOTICES.txt','CHANGELOG.md') | ForEach-Object { Join-Path $stage $_ }
Compress-Archive -LiteralPath $packageFiles -DestinationPath $archive -Force
$zipHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
$exeHash = (Get-FileHash -LiteralPath (Join-Path $stage 'TrackBridge.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'TrackBridge-SHA256SUMS.txt'),"$zipHash  $archiveName`n$exeHash  TrackBridge.exe`n")
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'TrackBridge-update.json'),([ordered]@{version=$version;archive=$archiveName;sha256=$zipHash} | ConvertTo-Json -Compress))
Write-Output ('Build: ' + $archive)
