# Publishes a new JondisClip version to GitHub Releases (ADR-0020). Friends'
# apps find it at their next update check and offer "Install update".
#
#   powershell -ExecutionPolicy Bypass -File tools\release.ps1 0.1.2
#   powershell -ExecutionPolicy Bypass -File tools\release.ps1 0.1.2 -Notes "Fixes recording on Intel graphics."
#
# It sets the version, builds the zip (tools\package.ps1), commits and tags
# "v<version>", pushes the current branch and the tag, and uploads the zip.
# Commit your changes first: it refuses to run with uncommitted work.
# Needs the GitHub CLI (`gh auth login` once).

param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Notes = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Invoke-Checked([string]$what, [scriptblock]$cmd) {
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$what failed" }
}

if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 0.1.2 (got '$Version')." }
$tag = "v$Version"
if (git status --porcelain) { throw "Commit or stash your changes first (git status shows uncommitted work)." }
if (git tag --list $tag) { throw "$tag already exists." }
$current = (Select-String Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
if ([version]$Version -le [version]$current -and (git tag --list "v$current")) {
    throw "$Version isn't newer than the released $current."
}

Write-Host "== Version $current -> $Version"
$utf8 = New-Object System.Text.UTF8Encoding($false)
function Set-Version([string]$file, [string]$pattern, [string]$replacement) {
    $path = Join-Path $root $file
    $text = [IO.File]::ReadAllText($path)
    $new = [regex]::new($pattern, 'Multiline').Replace($text, $replacement, 1)
    [IO.File]::WriteAllText($path, $new, $utf8)
}
Set-Version "Cargo.toml" '^version = ".+"' "version = `"$Version`""
Set-Version "ui\src-tauri\tauri.conf.json" '"version": ".+?"' "`"version`": `"$Version`""
Set-Version "ui\package.json" '"version": ".+?"' "`"version`": `"$Version`""

# Builds dist\JondisClip-<version>-win64.zip (and updates Cargo.lock).
& powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "package.ps1")
if ($LASTEXITCODE -ne 0) { throw "package.ps1 failed; nothing was published. Undo the version change with: git checkout ." }
$zip = Join-Path $root "dist\JondisClip-$Version-win64.zip"
if (-not (Test-Path $zip)) { throw "$zip wasn't built." }

Write-Host "== Commit, tag and push"
Invoke-Checked "git add" { git add Cargo.toml Cargo.lock ui/src-tauri/tauri.conf.json ui/package.json ui/package-lock.json }
if (git status --porcelain) {
    Invoke-Checked "git commit" { git commit -m "Release $tag" }
}
Invoke-Checked "git tag" { git tag $tag }
Invoke-Checked "git push" { git push origin HEAD $tag }

Write-Host "== GitHub release"
if ($Notes) {
    Invoke-Checked "gh release create" { gh release create $tag $zip --title "JondisClip $Version" --notes $Notes }
} else {
    Invoke-Checked "gh release create" { gh release create $tag $zip --title "JondisClip $Version" --generate-notes }
}
Write-Host "== Done. Share this link: https://github.com/sverre200/JondisClip/releases/latest"
