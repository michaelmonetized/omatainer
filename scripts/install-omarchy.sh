#!/usr/bin/env bash
# Install omatainer as an Omarchy-native app: binary, desktop, icon,
# Quickshell plugin, Hyprland rules, and keybinds.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOME="${HOME:-/home/michael}"
BIN="$HOME/.local/bin"
APP="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
PLUGIN="$HOME/.config/omarchy/plugins/omatainer"
HYPR_APPS="$HOME/.config/hypr/apps"

echo "==> building omatainer (release)"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

echo "==> installing binary + desktop"
install -Dm755 "$ROOT/target/release/omatainer" "$BIN/omatainer"
install -Dm644 "$ROOT/contrib/org.omarchy.omatainer.desktop" "$APP/org.omarchy.omatainer.desktop"
install -Dm644 "$ROOT/contrib/org.omarchy.omatainer.svg" "$ICON_DIR/org.omarchy.omatainer.svg"
mkdir -p "$HOME/.config/omatainer"

echo "==> installing Quickshell plugin"
mkdir -p "$PLUGIN"
# plugin validate forbids symlinks — copy the QML tree
cp -f "$ROOT/plugin/manifest.json" "$PLUGIN/manifest.json"
cp -f "$ROOT/plugin/BarWidget.qml" "$PLUGIN/BarWidget.qml"
cp -f "$ROOT/plugin/Service.qml" "$PLUGIN/Service.qml"
omarchy plugin validate "$PLUGIN"

echo "==> Hyprland window rules"
install -Dm644 "$ROOT/contrib/omatainer.lua" "$HYPR_APPS/omatainer.lua"

python3 - <<'PY'
from pathlib import Path
p = Path.home() / ".config/hypr/hyprland.lua"
text = p.read_text()
needle = 'require("hypr.apps.omatainer")'
if needle not in text:
    if 'require("hypr.apps.synchro")' in text:
        text = text.replace(
            'require("hypr.apps.synchro")',
            'require("hypr.apps.synchro")\nrequire("hypr.apps.omatainer")',
        )
    else:
        text += "\nrequire(\"hypr.apps.omatainer\")\n"
    p.write_text(text)
    print("patched hyprland.lua")
else:
    print("hyprland.lua already loads omatainer")
PY

python3 - <<'PY'
from pathlib import Path
p = Path.home() / ".config/hypr/bindings.lua"
text = p.read_text()
marker = "-- omatainer"
if marker not in text:
    block = '''
-- omatainer
o.bind("SUPER + O", "Omatainer", o.launch_sole("org.omarchy.omatainer", "omatainer"))
o.bind("SUPER + SHIFT + SPACE", "Omatainer play/stop", "omatainer ctl togglePlay")
'''
    p.write_text(text.rstrip() + "\n" + block)
    print("patched bindings.lua")
else:
    print("bindings.lua already has omatainer")
PY

python3 - <<'PY'
import json
from pathlib import Path
p = Path.home() / ".config/omarchy/shell.json"
data = json.loads(p.read_text())
plugins = data.setdefault("plugins", [])
if not any(x.get("id") == "omatainer" for x in plugins if isinstance(x, dict)):
    plugins.append({"id": "omatainer"})
right = data.setdefault("bar", {}).setdefault("layout", {}).setdefault("right", [])
if not any(x.get("id") == "omatainer" for x in right if isinstance(x, dict)):
    # park it next to the audio chip when present
    idx = next((i for i, x in enumerate(right) if x.get("id") == "omarchy.audio"), len(right))
    right.insert(idx + 1, {"id": "omatainer", "chipColor": "#89b4fa"})
p.write_text(json.dumps(data, indent=2) + "\n")
print("patched shell.json")
PY

python3 - <<'PY'
from pathlib import Path
p = Path.home() / ".config/omarchy/extensions/omarchy-menu.jsonc"
text = p.read_text()
if "omatainer" not in text:
    entry = '  "apps.omatainer": {"icon": "󰝚", "label": "Omatainer", "action": "omarchy-launch-or-focus org.omarchy.omatainer \'uwsm-app -- omatainer\'", "description": "DAW + dual-deck DJ"},\n'
    # insert after the opening brace
    brace = text.find("{")
    text = text[: brace + 1] + "\n" + entry + text[brace + 1 :]
    p.write_text(text)
    print("patched omarchy-menu.jsonc")
else:
    print("menu already has omatainer")
PY

HOOK_DIR="$HOME/.config/omarchy/hooks/theme-set.d"
mkdir -p "$HOOK_DIR"
cat > "$HOOK_DIR/omatainer-reload" <<'HOOK'
#!/bin/bash
# Theme files are watched by omatainer; this is a belt-and-braces ping.
omatainer ctl reload-theme >/dev/null 2>&1 || true
HOOK
chmod +x "$HOOK_DIR/omatainer-reload"

command -v update-desktop-database >/dev/null && update-desktop-database "$APP" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true

echo "==> validating hyprland"
hyprctl reload >/dev/null 2>&1 || true
hyprctl configerrors || true

echo "==> done"
echo "    omatainer            # launch"
echo "    SUPER+O              # launch or focus"
echo "    SUPER+SHIFT+SPACE    # play/stop from anywhere"
echo "    omarchy menu keybindings --print | grep -i omatainer"
