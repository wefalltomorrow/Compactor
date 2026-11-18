# Points bucket/compactor.json at the latest GitHub release.
# Requires the GitHub CLI (gh) with GH_TOKEN set when run in Actions.
param(
    [string]$Manifest = (Join-Path $PSScriptRoot '..\..\bucket\compactor.json')
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$text = [IO.File]::ReadAllText($Manifest)
$json = $text | ConvertFrom-Json
$repository = $json.homepage -replace '^https://github\.com/', ''

$tag = gh release view --repo $repository --json tagName --jq .tagName
if ($LASTEXITCODE -ne 0 -or -not $tag) {
    throw "Could not read the latest release of $repository."
}

$version = $tag -replace '^v', ''
if ($json.version -eq $version) {
    Write-Host "Scoop manifest is already at $version."
    return
}

$url = $json.autoupdate.architecture.'64bit'.url -replace '\$version', $version
$download = Join-Path ([IO.Path]::GetTempPath()) "Compactor-$version.exe"
Invoke-WebRequest -Uri $url -OutFile $download -UseBasicParsing
$hash = (Get-FileHash -Path $download -Algorithm SHA256).Hash.ToLowerInvariant()
Remove-Item $download

$text = $text -replace '("version":\s*")[^"]+"', "`${1}$version`""
$text = $text -replace '("url":\s*")[^"$]+"', "`${1}$url`""
$text = $text -replace '("hash":\s*")[^"]+"', "`${1}$hash`""
[IO.File]::WriteAllText($Manifest, $text, (New-Object Text.UTF8Encoding $false))

Write-Host "Updated Scoop manifest from $($json.version) to $version."
