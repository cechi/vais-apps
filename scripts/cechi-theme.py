#!/usr/bin/env python3
"""Generate the bundled "Cechi Dark" / "Cechi Light" themes (assets/themes/cechi.json)
from a DankMaterialShell (matugen) palette.

Usage: scripts/cechi-theme.py [path/to/dms-colors.json]
The default input is ~/.cache/DankMaterialShell/dms-colors.json. The output has no
font settings: fonts are a user preference (see `font_family` in the config file).
"""
import json
import os
import sys

src = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.cache/DankMaterialShell/dms-colors.json")
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out = os.path.join(root, "assets", "themes", "cechi.json")
d = json.load(open(src))


def darken(color, amount):
    """`color` (#rrggbb) moved `amount` (0-1) of the way to black."""
    rgb = [int(color[i:i + 2], 16) for i in (1, 3, 5)]
    return "#" + "".join(f"{round(v * (1 - amount)):02x}" for v in rgb)


def theme(name, mode, c, d16):
    dark = mode == "dark"
    fg_on_accent = c["on_primary"]
    colors = {
        "background": c["surface"],
        "foreground": c["on_surface"],
        "border": c["outline_variant"],
        "input.border": c["outline"],
        "ring": c["primary"],
        "caret": c["primary"],
        "link": c["primary"],
        "link.hover": c["primary_fixed"] if dark else c["primary_container"],
        "link.active": c["primary_fixed_dim"],
        "primary.background": c["primary"],
        "primary.foreground": fg_on_accent,
        "primary.hover.background": c["primary_fixed"] if dark else c["inverse_primary"],
        "primary.active.background": c["primary_fixed_dim"] if dark else c["on_primary_fixed_variant"],
        "secondary.background": c["secondary_container"],
        "secondary.foreground": c["on_secondary_container"],
        "secondary.hover.background": c["surface_container_highest"],
        "secondary.active.background": c["surface_variant"],
        "accent.background": c["primary_container"],
        "accent.foreground": c["on_primary_container"],
        "muted.background": c["surface_container_high"],
        "muted.foreground": c["on_surface_variant"],
        # Text selection: the accent at about a third opacity reads on any input.
        "selection.background": c["primary"] + ("59" if dark else "4d"),
        "list.background": c["surface"],
        "list.active.background": c["primary_container"],
        "list.active.border": c["primary"],
        "list.even.background": c["surface_container_low"],
        "list.head.background": c["surface_container_high"],
        "list.hover.background": c["surface_container_highest"],
        "table.background": c["surface"],
        "table.active.background": c["primary_container"],
        "table.active.border": c["primary"],
        "table.even.background": c["surface_container_low"],
        "table.head.background": c["surface_container_high"],
        "table.head.foreground": c["on_surface_variant"],
        "table.hover.background": c["surface_container_highest"],
        "table.row.border": c["outline_variant"],
        "title_bar.background": c["surface_container_low"],
        "title_bar.border": c["outline_variant"],
        "status_bar.background": c["surface_container_lowest"],
        "status_bar.border": c["outline_variant"],
        "tab_bar.background": c["surface_container_lowest"],
        "tab_bar.segmented.background": c["surface_container_high"],
        "tab.background": c["surface_container_lowest"],
        "tab.foreground": c["on_surface_variant"],
        "tab.active.background": c["primary_container"],
        "tab.active.foreground": c["on_primary_container"],
        "sidebar.background": c["surface_container_low"],
        "sidebar.foreground": c["on_surface"],
        "sidebar.border": c["outline_variant"],
        "sidebar.accent.background": c["primary_container"],
        "sidebar.accent.foreground": c["on_primary_container"],
        "sidebar.primary.background": c["primary"],
        "sidebar.primary.foreground": fg_on_accent,
        "popover.background": c["surface_container_high"],
        "popover.foreground": c["on_surface"],
        "accordion.background": c["surface_container_low"],
        "group_box.background": c["surface_container_low"],
        "group_box.foreground": c["on_surface"],
        "button.background": c["surface_container_highest"],
        "button.foreground": c["on_surface"],
        "button.hover.background": c["surface_bright"] if dark else c["surface_container_high"],
        "button.active.background": c["outline_variant"],
        "danger.background": c["error"],
        "danger.foreground": c["on_error"],
        "danger.hover.background": c["error_container"] if not dark else c["error"],
        "danger.active.background": c["error_container"],
        "warning.background": d16["color3"],
        "warning.foreground": d16["color0"] if dark else "#ffffff",
        "warning.hover.background": d16["color11"],
        "warning.active.background": d16["color11"],
        "success.background": d16["color2"],
        "success.foreground": d16["color0"] if dark else "#ffffff",
        "success.hover.background": d16["color10"],
        "success.active.background": d16["color10"],
        "info.background": c["tertiary"],
        "info.foreground": c["on_tertiary"],
        "info.hover.background": c["tertiary_fixed"],
        "info.active.background": c["tertiary_fixed_dim"],
        "progress.bar.background": c["primary"],
        "slider.background": c["primary"],
        "slider.thumb.background": c["primary"],
        "switch.background": c["primary"],
        "switch.thumb.background": fg_on_accent,
        "scrollbar.background": c["surface_container_low"],
        "scrollbar.thumb.background": c["outline"],
        "scrollbar.thumb.hover.background": c["primary"],
        "skeleton.background": c["surface_container_highest"],
        "drag.border": c["primary"],
        "drop_target.background": c["primary_container"],
        "window.border": c["outline_variant"],
        "base.red": d16["color1"], "base.red.light": d16["color9"],
        "base.green": d16["color2"], "base.green.light": d16["color10"],
        "base.yellow": d16["color3"], "base.yellow.light": d16["color11"],
        "base.blue": c["tertiary"], "base.blue.light": c["tertiary_fixed"],
        "base.magenta": d16["color5"], "base.magenta.light": d16["color13"],
        "base.cyan": d16["color6"], "base.cyan.light": d16["color14"],
    }
    # Syntax highlighting for the editor, in the same palette.
    syntax = {
        "keyword": {"color": c["primary"]},
        "function": {"color": c["tertiary_fixed_dim"] if dark else c["tertiary"]},
        "constructor": {"color": c["tertiary_fixed_dim"] if dark else c["tertiary"]},
        "type": {"color": d16["color6"]},
        "string": {"color": d16["color2"]},
        "string.escape": {"color": d16["color10"]},
        "string.regex": {"color": d16["color10"]},
        "string.special": {"color": d16["color1"]},
        "string.special.symbol": {"color": d16["color1"]},
        "number": {"color": d16["color3"]},
        "constant": {"color": d16["color3"]},
        "boolean": {"color": d16["color3"]},
        "comment": {"color": c["outline"], "font_style": "italic"},
        "comment.doc": {"color": c["outline"], "font_style": "italic"},
        "variable": {"color": c["on_surface"]},
        "variable.special": {"color": d16["color1"]},
        "property": {"color": c["on_surface_variant"]},
        "attribute": {"color": d16["color3"]},
        "tag": {"color": c["primary"]},
        "title": {"color": c["primary"], "font_weight": 700},
        "emphasis": {"font_style": "italic"},
        "emphasis.strong": {"font_weight": 700},
        "link_text": {"color": c["primary"]},
        "link_uri": {"color": c["outline"], "font_style": "italic"},
        "text.literal": {"color": d16["color5"]},
        "text.code.span": {"color": d16["color5"]},
        "embedded": {"color": c["on_surface"]},
        "punctuation": {"color": c["on_surface_variant"]},
        "operator": {"color": c["on_surface_variant"]},
    }
    highlight = {
        "editor.foreground": c["on_surface"],
        # A shade under the panels in the dark theme, so the editor stands out.
        "editor.background": darken(c["surface"], 0.45) if dark else c["surface"],
        "editor.active_line.background": c["surface_container_low"],
        "editor.line_number": c["outline"],
        "editor.active_line_number": c["on_surface"],
        "editor.invisible": c["outline_variant"] + "66",
        "conflict": c["error"],
        "created": d16["color2"],
        "hidden": c["outline"],
        "hint": c["secondary"],
        "modified": d16["color3"],
        "predictive": c["outline"],
        "warning": d16["color3"],
        "syntax": syntax,
    }
    # gpui-kit 0.8 built with syntect instead of tree-sitter reads these keys
    # spelled with underscores (its stub style struct lacks the renames), so
    # the editor colours are written both ways.
    for key, alias in (
        ("editor.background", "editor_background"),
        ("editor.foreground", "editor_foreground"),
        ("editor.active_line.background", "editor_active_line"),
        ("editor.line_number", "editor_line_number"),
        ("editor.active_line_number", "editor_active_line_number"),
        ("editor.invisible", "editor_invisible"),
    ):
        highlight[alias] = highlight[key]
    return {
        "name": name,
        "mode": mode,
        "radius": 8,
        "radius.lg": 12,
        "shadow": False,
        "colors": colors,
        "highlight": highlight,
    }


themes = []
for mode in ("dark", "light"):
    c = d["colors"][mode]
    d16 = {k: v[mode] for k, v in d["dank16"].items()}
    themes.append(theme(f"Cechi {mode.capitalize()}", mode, c, d16))
os.makedirs(os.path.dirname(out), exist_ok=True)
json.dump(
    {"name": "Cechi", "author": "generated from a DankMaterialShell palette", "themes": themes},
    open(out, "w"),
    indent=2,
)
open(out, "a").write("\n")
print("wrote", out, "with", [t["name"] for t in themes])
