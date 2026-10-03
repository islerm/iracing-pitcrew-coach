# /// script
# requires-python = ">=3.10"
# dependencies = ["piper-tts>=1.3"]
# ///
"""Long-lived Piper TTS sidecar for the coach. Same protocol as kokoro_tts.py.

Protocol (JSON lines):
  startup  -> {"ready": true} or {"ready": false, "error": "..."} (then exit 1)
  stdin    <- {"text": str, "out": path}   ("voice" is ignored: one model per process)
  stdout   -> {"ok": true} or {"ok": false, "error": "..."}
Nothing else is ever written to stdout; logs go to stderr.
"""
import argparse
import json
import sys
import wave


def reply(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, help="path to a Piper voice .onnx (with its .onnx.json next to it)")
    args = parser.parse_args()

    try:
        from piper import PiperVoice

        voice = PiperVoice.load(args.model)
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
            with wave.open(req["out"], "wb") as wav:
                voice.synthesize_wav(req["text"], wav)
            reply({"ok": True})
        except Exception as err:  # noqa: BLE001
            log(f"synthesis failed: {err}")
            reply({"ok": False, "error": str(err)})


if __name__ == "__main__":
    main()
