# /// script
# requires-python = ">=3.10"
# dependencies = ["kokoro-onnx", "soundfile", "numpy"]
# ///
"""Long-lived Kokoro TTS sidecar for the coach.

Protocol (JSON lines):
  startup  -> {"ready": true} or {"ready": false, "error": "..."} (then exit 1)
  stdin    <- {"text": str, "voice": str, "out": path}
  stdout   -> {"ok": true} or {"ok": false, "error": "..."}
Nothing else is ever written to stdout; logs go to stderr.
"""
import argparse
import json
import os
import sys


def reply(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--models", required=True, help="directory with kokoro-v1.0.onnx and voices-v1.0.bin")
    args = parser.parse_args()

    try:
        import soundfile as sf
        from kokoro_onnx import Kokoro

        model = os.path.join(args.models, "kokoro-v1.0.onnx")
        voices = os.path.join(args.models, "voices-v1.0.bin")
        kokoro = Kokoro(model, voices)
    except Exception as err:  # noqa: BLE001
        reply({"ready": False, "error": str(err)})
        sys.exit(1)

    reply({"ready": True})

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            voice = req["voice"]
            lang = "en-gb" if voice.startswith("b") else "en-us"
            samples, sr = kokoro.create(req["text"], voice=voice, lang=lang)
            sf.write(req["out"], samples, sr, subtype="PCM_16", format="WAV")
            reply({"ok": True})
        except Exception as err:  # noqa: BLE001
            log(f"synthesis failed: {err}")
            reply({"ok": False, "error": str(err)})


if __name__ == "__main__":
    main()
