# Downloads the coach's voice model into data/voice/.
#   scripts/setup-voice.ps1                    Piper, British female (en_GB-cori-high)
#   scripts/setup-voice.ps1 piper <voice>      another Piper voice, e.g. en_GB-jenny_dioco-medium
#   scripts/setup-voice.ps1 kokoro             Kokoro (slower, ~340 MB)
# Piper voices: https://huggingface.co/rhasspy/piper-voices
param([string]$Engine = "piper", [string]$Voice = "en_GB-cori-high")
$ErrorActionPreference = "Stop"

Set-Location (Join-Path $PSScriptRoot "..")

function Fetch($url, $path) {
    if ((Test-Path $path) -and ((Get-Item $path).Length -gt 0)) {
        Write-Host "$(Split-Path $path -Leaf) already present, skipping"
        return
    }
    Write-Host "Downloading $(Split-Path $path -Leaf) ..."
    Invoke-WebRequest -Uri $url -OutFile "$path.part"
    Move-Item -Force "$path.part" $path
}

switch ($Engine) {
    "piper" {
        # en_GB-cori-high -> en/en_GB/cori/high/en_GB-cori-high
        $parts = $Voice.Split("-")
        $locale = $parts[0]
        $quality = $parts[-1]
        $speaker = ($parts[1..($parts.Length - 2)] -join "-")
        $url = "https://huggingface.co/rhasspy/piper-voices/resolve/main/$($locale.Split('_')[0])/$locale/$speaker/$quality/$Voice"
        $dest = "data/voice/piper"
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        Fetch "$url.onnx" (Join-Path $dest "$Voice.onnx")
        Fetch "$url.onnx.json" (Join-Path $dest "$Voice.onnx.json")
    }
    "kokoro" {
        $base = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0"
        $dest = "data/voice"
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        foreach ($name in @("kokoro-v1.0.onnx", "voices-v1.0.bin")) { Fetch "$base/$name" (Join-Path $dest $name) }
    }
    default { Write-Error "Unknown engine '$Engine': use piper or kokoro"; exit 2 }
}

if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    Write-Host ""
    Write-Host "uv is not installed. It runs the Python voice sidecar and its dependencies."
    Write-Host 'Install it with:  powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"'
    exit 1
}

Write-Host "Voice setup done. Models are in $dest"
