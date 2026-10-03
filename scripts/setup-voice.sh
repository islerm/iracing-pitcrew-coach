#!/usr/bin/env sh
# Downloads the coach's voice model into data/voice/.
#   scripts/setup-voice.sh                     Piper, British female (en_GB-cori-high)
#   scripts/setup-voice.sh piper <voice>       another Piper voice, e.g. en_GB-jenny_dioco-medium
#   scripts/setup-voice.sh kokoro              Kokoro (slower, ~340 MB)
# Piper voices: https://huggingface.co/rhasspy/piper-voices
set -eu

cd "$(dirname "$0")/.."
ENGINE="${1:-piper}"

fetch() { # url dest
  if [ -s "$2" ]; then
    echo "$(basename "$2") already present, skipping"
    return
  fi
  echo "Downloading $(basename "$2") ..."
  curl -L --fail --progress-bar -o "$2.part" "$1"
  mv "$2.part" "$2"
}

case "$ENGINE" in
  piper)
    VOICE="${2:-en_GB-cori-high}"
    # en_GB-cori-high -> en/en_GB/cori/high/en_GB-cori-high
    LOCALE="${VOICE%%-*}"
    REST="${VOICE#*-}"
    SPEAKER="${REST%-*}"
    QUALITY="${REST##*-}"
    URL="https://huggingface.co/rhasspy/piper-voices/resolve/main/${LOCALE%%_*}/$LOCALE/$SPEAKER/$QUALITY/$VOICE"
    DEST="data/voice/piper"
    mkdir -p "$DEST"
    fetch "$URL.onnx" "$DEST/$VOICE.onnx"
    fetch "$URL.onnx.json" "$DEST/$VOICE.onnx.json"
    ;;
  kokoro)
    BASE="https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0"
    DEST="data/voice"
    mkdir -p "$DEST"
    fetch "$BASE/kokoro-v1.0.onnx" "$DEST/kokoro-v1.0.onnx"
    fetch "$BASE/voices-v1.0.bin" "$DEST/voices-v1.0.bin"
    ;;
  *)
    echo "Unknown engine '$ENGINE': use piper or kokoro" >&2
    exit 2
    ;;
esac

if ! command -v uv >/dev/null 2>&1; then
  echo
  echo "uv is not installed. It runs the Python voice sidecar and its dependencies."
  echo "Install it with:  curl -LsSf https://astral.sh/uv/install.sh | sh   (or: brew install uv)"
  exit 1
fi

echo "Voice setup done. Models are in $DEST"
