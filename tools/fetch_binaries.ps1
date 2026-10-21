# Downloads the third-party binaries that are not stored in the repository:
# llama.cpp (llama-server and DLLs) and DirectML. Skips a set that is already present.
param([switch]$Force)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.IO.Compression.FileSystem

$root = Split-Path $PSScriptRoot -Parent
$llamaDir = Join-Path $root "src-tauri\resources\llama"
$dmlDir = Join-Path $root "src-tauri\dml"

$llamaUrl = "https://github.com/ggml-org/llama.cpp/releases/download/b11741/llama-b11741-bin-win-vulkan-x64.zip"
$llamaSha = "1981b9898b68dc6637077d6c618effaa350f5939ee75e9a4d803508708413ab5"
$llamaFiles = @(
    "ggml-base.dll", "ggml-cpu-alderlake.dll", "ggml-cpu-cannonlake.dll", "ggml-cpu-cascadelake.dll",
    "ggml-cpu-cooperlake.dll", "ggml-cpu-haswell.dll", "ggml-cpu-icelake.dll", "ggml-cpu-ivybridge.dll",
    "ggml-cpu-piledriver.dll", "ggml-cpu-sandybridge.dll", "ggml-cpu-sapphirerapids.dll",
    "ggml-cpu-skylakex.dll", "ggml-cpu-sse42.dll", "ggml-cpu-x64.dll", "ggml-cpu-zen4.dll",
    "ggml-vulkan.dll", "ggml.dll", "libomp.dll", "llama-common.dll", "llama-server-impl.dll",
    "llama-server.exe", "llama.dll", "mtmd.dll"
)

$dmlUrl = "https://www.nuget.org/api/v2/package/Microsoft.AI.DirectML/1.15.4"
$dmlSha = "4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9"
$dmlEntry = "bin/x64-win/DirectML.dll"

function Get-Archive($url, $sha, $tmp) {
    $file = Join-Path $tmp "archive.zip"
    Write-Host "Downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $file -UseBasicParsing
    if ((Get-FileHash $file -Algorithm SHA256).Hash -ne $sha) { throw "SHA256 mismatch for $url" }
    $file
}

# Extracts the entries (name in the archive -> file path) from a zip file.
function Expand-Entries($zipPath, $entries) {
    $zip = [IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        foreach ($name in $entries.Keys) {
            $entry = $zip.GetEntry($name)
            if (-not $entry) { throw "$name not found in archive" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $entries[$name], $true)
        }
    } finally { $zip.Dispose() }
}

function Test-Present($dir, $names) {
    -not $Force -and -not ($names | Where-Object { -not (Test-Path (Join-Path $dir $_)) })
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("murmur-fetch-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    if (Test-Present $llamaDir $llamaFiles) {
        Write-Host "llama.cpp: already present"
    } else {
        $zip = Get-Archive $llamaUrl $llamaSha $tmp
        New-Item -ItemType Directory -Force -Path $llamaDir | Out-Null
        $entries = @{}
        foreach ($f in $llamaFiles) { $entries[$f] = Join-Path $llamaDir $f }
        Expand-Entries $zip $entries
        Write-Host "llama.cpp: $($llamaFiles.Count) files in $llamaDir"
    }

    if (Test-Present $dmlDir @("DirectML.dll")) {
        Write-Host "DirectML: already present"
    } else {
        $zip = Get-Archive $dmlUrl $dmlSha $tmp
        New-Item -ItemType Directory -Force -Path $dmlDir | Out-Null
        Expand-Entries $zip @{ $dmlEntry = (Join-Path $dmlDir "DirectML.dll") }
        Write-Host "DirectML: DirectML.dll in $dmlDir"
    }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
