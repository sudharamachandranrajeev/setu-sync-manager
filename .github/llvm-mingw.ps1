# The C compiler cgo uses to build Setu's sync engine on Windows: llvm-mingw, pinned by version
# and checksum. Sets SETU_CGO_CC for the following steps (see engine/build.rs).
$ErrorActionPreference = 'Stop'
$version = '20261117'
$sha256 = '4d002e330a0f703625aa5ccbc37e96f8f824413e163bfc7336774aabd2c3a412'
$name = "llvm-mingw-$version-ucrt-x86_64"
$zip = Join-Path $env:RUNNER_TEMP "$name.zip"
Invoke-WebRequest "https://github.com/mstorsjo/llvm-mingw/releases/download/$version/$name.zip" -OutFile $zip
$got = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
if ($got -ne $sha256) { throw "llvm-mingw checksum mismatch: $got" }
Expand-Archive $zip $env:RUNNER_TEMP
"SETU_CGO_CC=$(Join-Path $env:RUNNER_TEMP "$name\bin\x86_64-w64-mingw32-clang.exe")" | Out-File -Append -Encoding utf8 $env:GITHUB_ENV
