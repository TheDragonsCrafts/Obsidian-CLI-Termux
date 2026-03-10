$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$ndkRoot = Join-Path $repoRoot ".tooling\android-ndk-r29"
$clang = Join-Path $ndkRoot "toolchains\llvm\prebuilt\windows-x86_64\bin\aarch64-linux-android24-clang.cmd"

if (-not (Test-Path $clang)) {
    throw "No se encontró el linker del NDK en '$clang'. Descarga y extrae el NDK oficial en '.tooling\\android-ndk-r29' o ajusta la ruta en este script."
}

Push-Location $repoRoot
try {
    $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = (Resolve-Path $clang).Path
    cargo build --release --target aarch64-linux-android
}
finally {
    Pop-Location
}

$artifact = Join-Path $repoRoot "target\aarch64-linux-android\release\obsidian"
Write-Host ""
Write-Host "Artefacto generado:"
Write-Host "  $artifact"
