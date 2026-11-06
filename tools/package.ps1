# Builds a portable JondisClip zip to share: unzip anywhere, run jondisclip.exe.
#
#   powershell -ExecutionPolicy Bypass -File tools\package.ps1
#
# Output: dist\JondisClip-<version>-win64.zip (and the unpacked dist\JondisClip).
# It builds into target\package with the C runtime linked in (+crt-static), so
# friends don't need the Visual C++ redistributable, and so the exes you're
# running from target\release are never locked or replaced.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

$version = (Select-String Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "JondisClip-$version-win64"
$stage = Join-Path $root "dist\JondisClip"
$zip = Join-Path $root "dist\$name.zip"
$ffmpeg = Join-Path $root "third_party\ffmpeg\9.0"

Write-Host "== JondisClip $version"
if (-not (Test-Path "$ffmpeg\bin\avcodec-63.dll")) { throw "FFmpeg isn't in $ffmpeg (see CLAUDE.md)." }

Write-Host "== UI bundle"
npm --prefix ui run build
if ($LASTEXITCODE -ne 0) { throw "npm run build failed" }

Write-Host "== Release build (static C runtime)"
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release --target-dir target\package -p jondisclip -p jondisclip-ui -p jondisclip-export
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
Remove-Item Env:\RUSTFLAGS
$bin = Join-Path $root "target\package\release"

Write-Host "== Staging $stage"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force "$stage\licenses" | Out-Null
foreach ($f in "jondisclip.exe", "jondisclip-ui.exe", "jondisclip-export.exe") {
    Copy-Item "$bin\$f" $stage
}
# Only the FFmpeg libraries the exes import (avfilter/avdevice aren't used).
foreach ($f in "avcodec-63.dll", "avformat-63.dll", "avutil-61.dll", "swscale-10.dll", "swresample-7.dll") {
    Copy-Item "$ffmpeg\bin\$f" $stage
}
Copy-Item "$ffmpeg\LICENSE.txt" "$stage\licenses\FFmpeg-LGPL.txt"
Copy-Item "tools\package\README.txt" $stage
Copy-Item "tools\package\THIRD-PARTY-NOTICES.txt" "$stage\licenses"

Write-Host "== Checking imports"
$dumpbin = Get-ChildItem "C:\Program Files*\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty FullName
if ($dumpbin) {
    foreach ($exe in Get-ChildItem "$stage\*.exe") {
        $deps = & $dumpbin /nologo /dependents $exe.FullName | Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim() }
        if ($deps -match '^(VCRUNTIME|MSVCP)') { throw "$($exe.Name) still needs the Visual C++ runtime: $($deps -match '^(VCRUNTIME|MSVCP)')" }
        foreach ($d in $deps -match '^(av|sw)') {
            if (-not (Test-Path (Join-Path $stage $d))) { throw "$($exe.Name) needs $d, which isn't in the package" }
        }
    }
    Write-Host "   no Visual C++ runtime needed; all FFmpeg DLLs present"
} else {
    Write-Warning "dumpbin not found: imports not checked"
}

Write-Host "== Zipping"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal
$mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host "== Done: $zip ($mb MB)"
