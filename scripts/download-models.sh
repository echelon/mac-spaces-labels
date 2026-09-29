#!/bin/sh
# Downloads Spaces Labels' local models into the app's models folder.
# Resumes partial files and retries stalled transfers (Hugging Face's CDN
# regularly stalls mid-file). Usage: download-models.sh [default|vl4b]
set -eu

MODELS="$HOME/Library/Application Support/io.echelon.spaces-labels/models"
mkdir -p "$MODELS"

fetch() { # repo file
  if [ -f "$MODELS/$2" ]; then
    echo "have $2"
    return
  fi
  echo "downloading $2 from $1"
  attempt=0
  # Give up on a transfer slower than 100 KB/s for 20 s and resume it.
  until curl -fL -C - --speed-limit 100000 --speed-time 20 \
      -o "$MODELS/$2.part" "https://huggingface.co/$1/resolve/main/$2"; do
    attempt=$((attempt + 1))
    [ "$attempt" -ge 40 ] && { echo "giving up on $2" >&2; exit 1; }
    echo "  stalled, resuming ($attempt)"
    sleep 1
  done
  mv "$MODELS/$2.part" "$MODELS/$2"
}

case "${1:-default}" in
  default)
    fetch Qwen/Qwen3-VL-2B-Instruct-GGUF Qwen3VL-2B-Instruct-Q8_0.gguf
    fetch Qwen/Qwen3-VL-2B-Instruct-GGUF mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf
    fetch unsloth/Qwen3-4B-Instruct-2507-GGUF Qwen3-4B-Instruct-2507-Q4_K_M.gguf
    ;;
  vl4b)
    fetch Qwen/Qwen3-VL-4B-Instruct-GGUF Qwen3VL-4B-Instruct-Q4_K_M.gguf
    fetch Qwen/Qwen3-VL-4B-Instruct-GGUF mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf
    ;;
  *)
    echo "usage: $0 [default|vl4b]" >&2
    exit 2
    ;;
esac
command -v llama-server >/dev/null || echo "note: llama-server not found; run: brew install llama.cpp"
echo "models in $MODELS"
