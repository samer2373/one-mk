import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi


binary = Path(sys.argv[1]).resolve()
output = Path(sys.argv[2])
output.mkdir(parents=True, exist_ok=True)
report = {"transport": "Linux AT-SPI via GI Atspi on an isolated D-Bus session", "screens": {}}
subprocess.run(["gdbus", "call", "--session", "--dest", "org.a11y.Bus", "--object-path", "/org/a11y/bus", "--method", "org.freedesktop.DBus.Properties.Set", "org.a11y.Status", "IsEnabled", "<true>"], check=True, capture_output=True)


def visit(node, rows, depth=0):
    if depth > 16 or len(rows) > 2000:
        return
    try:
        row = {"role": node.get_role_name(), "name": node.get_name(), "states": [state.value_nick for state in node.get_state_set().get_states()], "interfaces": list(node.get_interfaces())}
        try:
            value = node.get_value_iface()
            if value:
                row["value"] = value.get_current_value()
        except Exception as error:
            row["value_error"] = str(error)
        try:
            text = node.get_text_iface()
            if text:
                row["text"] = text.get_text(0, -1)
        except Exception as error:
            row["text_error"] = str(error)
        rows.append(row)
        for index in range(node.get_child_count()):
            visit(node.get_child_at_index(index), rows, depth + 1)
    except Exception as error:
        rows.append({"error": str(error)})


with tempfile.TemporaryDirectory(prefix="codex-atspi-") as directory:
    home = Path(directory)
    status = home / "status"
    status.write_text("ok\nname\tCodex reference\nid\tabcd-ef01\nport\t24800\npeer\t1111-2222\tleft\t1\t1\tauto\tStudio PC\nlink\t1111-2222\t4200\t0\t900\n")
    for tab in ["files", "keys", "keys-mouse", "layout"]:
        environment = dict(os.environ, ONE_MK_APP_NO_SETUP="1", ONE_MK_APP_STATUS=str(status), ONE_MK_APP_TAB=tab, ONE_MK_APP_SELECT="Studio PC", ONE_MK_APP_RTT="1111-2222=" + ",".join(["4.2"] * 120), WINIT_UNIX_BACKEND="x11")
        if tab.startswith("keys"):
            environment.update(ONE_MK_APP_TAB="keys", ONE_MK_APP_KEY="capslock", ONE_MK_APP_KEY_MORE="1")
        if tab == "keys-mouse":
            environment.update(ONE_MK_APP_KEYS="mouse", ONE_MK_APP_SCROLL="250")
        environment.pop("WAYLAND_DISPLAY", None)
        with (output / f"atspi-{tab}.log").open("w") as log:
            process = subprocess.Popen([str(binary), "--home", str(home)], env=environment, stdout=log, stderr=log)
            try:
                rows = []
                for attempt in range(30):
                    time.sleep(0.5)
                    desktop = Atspi.get_desktop(0)
                    for index in range(desktop.get_child_count()):
                        application = desktop.get_child_at_index(index)
                        if application.get_process_id() == process.pid:
                            visit(application, rows)
                    if len(rows) > 10:
                        break
                report["screens"][tab] = rows
            finally:
                process.terminate()
                process.wait(timeout=10)
report["named_controls"] = [row for rows in report["screens"].values() for row in rows if row.get("name") and row.get("role") in ["button", "push button", "switch", "toggle button", "check box", "radio button", "entry", "page tab"]]
(output / "atspi.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({"nodes": {tab: len(rows) for tab, rows in report["screens"].items()}, "named_controls": len(report["named_controls"])}))
