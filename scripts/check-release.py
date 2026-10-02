#!/usr/bin/env python3
"""Validate public, non-secret release metadata without touching the network."""
import base64
import pathlib
import plistlib
import json
import subprocess
from urllib.parse import urlparse
root = pathlib.Path(__file__).resolve().parents[1]
package = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--offline", "--locked", "--format-version=1"], cwd=root))["packages"][0]
with (root / "assets/Info.plist").open("rb") as f:
    info = plistlib.load(f)
with (root / "assets/UpdateConfig.plist").open("rb") as f:
    update = plistlib.load(f)
assert package["version"] == info["CFBundleShortVersionString"], "Cargo/plist version mismatch"
assert int(info["CFBundleVersion"]) > 0
assert len(base64.b64decode(update["SUPublicEDKey"], validate=True)) == 32
assert urlparse(update["SUFeedURL"]).scheme == "https"
assert update["SUVerifyUpdateBeforeExtraction"] and update["SURequireSignedFeed"]
assert update["SUEnableSystemProfiling"] is False
assert "SUFeedURL" not in info, "Development builds must not inherit upstream updates"
print(f"Release metadata valid: {package['version']} (build {info['CFBundleVersion']})")
