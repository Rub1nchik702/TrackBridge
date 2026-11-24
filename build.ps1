param([string]$OutputDirectory = (Join-Path $PSScriptRoot 'dist'))
$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot 'rust/Cargo.toml'
$artifacts = & cargo build --release --locked --manifest-path $manifest --message-format=json-render-diagnostics
if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
$binary = $artifacts | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object {
    $_.reason -eq 'compiler-artifact' -and $_.target.name -ceq 'TrackBridge' -and $_.executable
} | Select-Object -Last 1
if (!$binary) { throw 'Cargo did not produce the TrackBridge executable.' }
$version = [Diagnostics.FileVersionInfo]::GetVersionInfo($binary.executable).ProductVersion
if (!$version) { throw 'Application version is missing.' }
$stage = Join-Path $OutputDirectory 'TrackBridge'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$exe = Join-Path $stage 'TrackBridge.exe'
Copy-Item -LiteralPath $binary.executable -Destination $exe -Force
$documents = @('README.md','README.ru.md','LICENSE.txt','THIRD-PARTY-NOTICES.txt','CHANGELOG.md','TRADEMARKS.md')
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
