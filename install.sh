#!/bin/sh
# Installs the global Zed tasks, helper script and keybindings used by the
# .NET Run & Debug extension. Safe to re-run: entries are replaced, not duplicated.
set -e

here=$(cd "$(dirname "$0")" && pwd)
zed_config="${ZED_CONFIG_DIR:-$HOME/.config/zed}"
script="$zed_config/dotnet/dotnet-zed.sh"

mkdir -p "$zed_config/dotnet"
cp "$here/scripts/dotnet-zed.sh" "$script"
chmod +x "$script"
echo "✓ script  → $script"

python3 - "$here" "$zed_config" "$script" <<'PY'
import json, os, re, sys

here, zed_config, script = sys.argv[1:]

def load(path, default):
    if not os.path.exists(path):
        return default
    text = open(path).read()
    # Zed config files are JSONC: drop // comments and trailing commas.
    text = re.sub(r'^\s*//.*$', '', text, flags=re.M)
    text = re.sub(r',(\s*[\]}])', r'\1', text)
    try:
        return json.loads(text) if text.strip() else default
    except json.JSONDecodeError as e:
        sys.exit(f"✗ could not parse {path} ({e}); add the entries from config/ by hand")

def save(path, data):
    with open(path, "w") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")

# tasks.json: replace every task that calls the helper script
tasks_path = os.path.join(zed_config, "tasks.json")
ours = json.loads(open(os.path.join(here, "config/tasks.json")).read().replace("__SCRIPT__", script))
def is_ours(task):
    return any(str(arg).endswith("dotnet-zed.sh") for arg in task.get("args", []))
tasks = [t for t in load(tasks_path, []) if not is_ours(t)] + ours
save(tasks_path, tasks)
print(f"✓ tasks   → {tasks_path}")

# keymap.json: drop our previous bindings, then add the current ones
keymap_path = os.path.join(zed_config, "keymap.json")
our_bindings = json.load(open(os.path.join(here, "config/keymap.json")))[0]["bindings"]
keymap = load(keymap_path, [])
def spawns_dotnet_task(action):
    return (isinstance(action, list) and len(action) == 2 and action[0] == "task::Spawn"
            and str(action[1].get("task_name", "")).startswith("dotnet:"))
for block in keymap:
    bindings = block.get("bindings", {})
    for key in [k for k, action in bindings.items() if k in our_bindings or spawns_dotnet_task(action)]:
        del bindings[key]
keymap = [b for b in keymap if b.get("bindings") or b.get("context") != "Workspace"]
keymap.append({"context": "Workspace", "bindings": our_bindings})
save(keymap_path, keymap)
print(f"✓ keymap  → {keymap_path}")
PY

cat <<EOF

One-time step, in Zed's settings.json under "languages":
  "CSharp": { "debuggers": ["netcoredbg"] }
EOF
