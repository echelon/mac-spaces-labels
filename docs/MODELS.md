# Local models

Spaces Labels uses two small open-weight models, run locally with llama.cpp on
Apple silicon (Metal). Nothing is sent to a hosted model: the servers bind to
`127.0.0.1` on a random port, and the only network use is downloading the
weights once. Both models are optional; without them the app still shows
labels, apps, tabs, agents and projects, just no screenshot descriptions or
AI-written subtitles.

## The models

| Role | Model | Files (put both in the models folder) | Size | Memory while loaded |
| --- | --- | --- | --- | --- |
| Screenshot descriptions ("what is this window doing") | Qwen3-VL-2B-Instruct, Q8_0 | `Qwen3VL-2B-Instruct-Q8_0.gguf`, `mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf` | 1.8 GB + 0.45 GB | ~3.7 GB |
| Desktop naming (the subtitle / task) | Qwen3-4B-Instruct-2507, Q4_K_M | `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` | 2.5 GB | ~2.5–3.5 GB |
| Optional: sharper screenshots | Qwen3-VL-4B-Instruct, Q4_K_M | `Qwen3VL-4B-Instruct-Q4_K_M.gguf`, `mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf` | 2.5 GB + 0.45 GB | ~4.6 GB |

Sources (Hugging Face):

- Qwen3-VL-2B: <https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF> (official Qwen repo)
- Qwen3-4B-Instruct-2507: <https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF>
  (Qwen publishes no official GGUF of this one; `lmstudio-community` and
  `bartowski` have equivalent files)
- Qwen3-VL-4B: <https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF>

Models folder: `~/Library/Application Support/io.echelon.spaces-labels/models/`

Which file the app uses is decided by what is in that folder
(`crates/spaces_labels_app/src/vision.rs`: `VISION_MODELS`, `TEXT_MODELS`):
screenshots use the 2B vision model if present, else the 4B; names use
Qwen3-4B-Instruct if present, else whichever vision model is loaded.

### Why these

Measured on an M4 Pro (48 GB), against this machine's real desktops:

| Model | Screenshot (1024 image tokens) | Name a desktop | Notes |
| --- | --- | --- | --- |
| SmolVLM2-500M | ~1 s | — | Fast but says little more than "working on a project". Rejected. |
| Qwen3-VL-2B | 1.0–1.6 s | 0.4–0.9 s | Reads terminal and page text well. Names vague ("Artcraft Development"). |
| Qwen3-VL-4B | 2.1–2.4 s | 0.5–1.3 s | More precise descriptions; names drift to the latest step. |
| Qwen3-4B-Instruct-2507 (text) | — | 0.5–1.4 s | Best names ("Retire Legacy Models", "Improve Thumbnail Performance"). |

Screenshots are frequent, so the fast 2B describes them; names are rare and
shown on screen, so the stronger 4B text model writes them.

## Installing

1. llama.cpp (provides `llama-server`):

   ```sh
   brew install llama.cpp
   ```

2. The weights:

   ```sh
   make models            # the two default models
   make models-vl4b       # optional: the 4B vision model
   ```

   `scripts/download-models.sh` downloads with resume and retries a stalled
   transfer, because Hugging Face's CDN regularly stalls mid-file. By hand:

   ```sh
   M="$HOME/Library/Application Support/io.echelon.spaces-labels/models"
   mkdir -p "$M" && cd "$M"
   curl -L -C - -O https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/main/Qwen3VL-2B-Instruct-Q8_0.gguf
   curl -L -C - -O https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/main/mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf
   curl -L -C - -O https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf
   ```

   Re-run a `curl` that stalls; `-C -` continues where it stopped.

3. Menu bar → **Describe windows with local AI** must be on (it is by
   default). Screenshot descriptions also need Screen Recording permission.

## How the app runs them

Each model runs as its own `llama-server` child process, started on first
use:

```sh
llama-server -m <weights> [--mmproj <projector> --image-max-tokens 1024] \
  --host 127.0.0.1 --port <random> --ctx-size 4096 --parallel 1 \
  --threads 4 --prio -1 --no-webui --reasoning off
```

- **Politeness.** One request at a time with a 3 s cooldown; windows on the
  desktop you are looking at first; each window re-described at most every
  60 s (showing) or 5 min (other desktops) unless its title changes; desktop
  names are cached and redone only when their projects change (at most every
  30 s) or after 10 minutes. Everything pauses under serious thermal pressure,
  in Low Power Mode, or when the menu toggle is off.
- **Memory.** The vision server stops after 10 idle minutes, the naming server
  after 2. Both hold their weights in GPU (wired) memory while running.
- **Lifetime.** The app owns each `llama-server` process and kills it directly
  when it stops a server. A small watchdog shell per server kills it if the
  app dies (even by crashing), and at startup the app kills any leftover
  `llama-server` of its own (parent gone, command line pointing at the models
  folder).

> An earlier build (before 2026-09-29) stopped idle servers by killing a shell
> wrapper, which left the `llama-server` itself running. Each idle shutdown
> leaked ~3 GB of wired memory. Current builds fix and clean this up
> automatically.

Check what is running at any time:

```sh
ps -axo pid,ppid,rss,etime,command | grep '[l]lama-server'
```

Expect at most two (one per model), each with the app as parent.

## Running a model by hand

Useful for trying prompts or other models. Use a port the app will not pick:

```sh
M="$HOME/Library/Application Support/io.echelon.spaces-labels/models"
llama-server -m "$M/Qwen3VL-2B-Instruct-Q8_0.gguf" \
  --mmproj "$M/mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf" \
  --image-max-tokens 1024 --host 127.0.0.1 --port 18733 --no-webui
```

Describe a window (any window, on any desktop; needs Screen Recording for the
terminal):

```sh
screencapture -x -o -t jpg -l <window-id> /tmp/w.jpg && sips -Z 1024 /tmp/w.jpg
IMG=$(base64 -i /tmp/w.jpg)
curl -s http://127.0.0.1:18733/v1/chat/completions -H 'Content-Type: application/json' -d @- <<EOF | jq -r '.choices[0].message.content'
{"messages":[{"role":"user","content":[
  {"type":"image_url","image_url":{"url":"data:image/jpeg;base64,$IMG"}},
  {"type":"text","text":"In one sentence, what is the user doing in this window?"}]}],
 "max_tokens":60,"temperature":0}
EOF
```

Window ids: `cargo run --release -p app-context --bin context_probe -- --windows`.
The exact naming input for a desktop is the `input` field of each rename in
`feedback.jsonl`, and the prompts are in `crates/spaces_labels_app/src/naming.rs`.

Stop the manual server with Ctrl-C.

## Removing

Delete the files from the models folder (or turn the menu toggle off). The
app falls back to what remains and simply skips what needs a missing model.
