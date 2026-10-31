#!/usr/bin/env python3
"""Regenerate Wave 6 live and explicitly synthetic UI evidence in isolation."""

import importlib.util
import os
from pathlib import Path
import select
import subprocess
import tempfile

from PIL import Image


def main():
    assert os.environ.get("ONE_MK_ISOLATED_AUDIT") == "1", "use audit-wave6-isolated.sh"
    repository = Path(__file__).resolve().parents[1]
    specification = importlib.util.spec_from_file_location("cli_audit", repository / "scripts/audit-wave6.py")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    target = Path(os.environ["CARGO_TARGET_DIR"])
    output = repository / "docs/screens/wave6"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="w6ui-") as temporary:
        audit = module.Audit(target / "debug/one-mk", Path(temporary))
        xvfb = None
        try:
            first, second = audit.home("a"), audit.home("b")
            xvfb = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1800x1400x24", "-nolisten", "tcp"],
                                    env=audit.env(first), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            assert select.select([xvfb.stdout], [], [], 10)[0], "private Xvfb startup timed out"
            display = xvfb.stdout.readline().decode().strip()
            assert display.isdigit() and int(display) > 1
            for home in [first, second]:
                audit.daemon(home)
            audit.pair(first, second)

            def screenshot(name, tab="", fixture=None, size="980x660", **extras):
                raw = Path(temporary) / (name + ".ppm")
                environment = dict(audit.env(first), DISPLAY=":" + display,
                                   ONE_MK_APP_SHOT=str(raw), ONE_MK_APP_SHOT_AFTER="3",
                                   ONE_MK_APP_TAB=tab, ONE_MK_APP_SIZE=size, **extras)
                if fixture:
                    environment["ONE_MK_APP_STATUS"] = str(fixture)
                result = subprocess.run([str(target / "debug/one-mk-app"), "--home", str(first)],
                                        env=environment, capture_output=True, text=True, timeout=25)
                assert result.returncode == 0 and raw.exists(), (name, result.stderr)
                with Image.open(raw) as image:
                    image.save(output / (name + ".png"), optimize=True)
                print("PASS", name, flush=True)

            for tab in ["", "files", "layout", "keys", "health", "advanced"]:
                screenshot("live-" + (tab or "desk"), tab)
            audit.cli(first, "broadcast", "on")
            screenshot("live-broadcast-small", size="760x540")
            audit.cli(first, "broadcast", "off")
            fixture = Path(temporary) / "synthetic.status"
            long_name = "Long workstation name with several words and no available edge"
            fixture.write_text("ok\nname\tAudit host\nid\taaaa-bbbb\nport\t24900\n" +
                               "broadcast\t" + long_name + "\n" +
                               "peer\t1111-2222\tleft\t1\t1\tauto\t" + long_name + "\n" +
                               "peer\t3333-4444\tright\t2\t0\toff\tOffline computer\n" +
                               "link\t1111-2222\t4200\t0\t900\n" +
                               "screen\tme\t0\t0\t1920\t1080\n" +
                               "screen\t1111-2222\t0\t0\t1920\t1080\n")
            screenshot("fixture-long-broadcast-small", fixture=fixture, size="760x540")
            screenshot("fixture-latency-small", fixture=fixture, size="760x540",
                       ONE_MK_APP_SELECT=long_name, ONE_MK_APP_RTT="1111-2222=4,5,100,6,7+2")
            screenshot("fixture-layout-small", "layout", fixture=fixture, size="760x540")
            screenshot("fixture-keys-small", "keys", fixture=fixture, size="760x540")
        finally:
            audit.close()
            if xvfb:
                xvfb.terminate()
                xvfb.wait(timeout=5)
                xvfb.stdout.close()


if __name__ == "__main__":
    main()
