"""Generate the IMK bundle metadata without Xcode project or string substitution."""
import plistlib
import sys
from pathlib import Path

app, name, bundle_id, version, minimum = sys.argv[1:]
bundle_version = version.split("-", 1)[0].split("+", 1)[0]
mode_id = bundle_id + ".Zhuyin"
metadata = {
    "CFBundleDevelopmentRegion": "zh_TW",
    "CFBundleDisplayName": name,
    "CFBundleName": name,
    "CFBundleIdentifier": bundle_id,
    "CFBundleExecutable": "zoo-type",
    "CFBundlePackageType": "APPL",
    "CFBundleVersion": bundle_version,
    "CFBundleShortVersionString": bundle_version,
    "ZooTypeVersion": version,
    "CFBundleIconFile": "zoo-type.icns",
    "LSMinimumSystemVersion": minimum,
    "LSBackgroundOnly": True,
    "LSUIElement": True,
    "NSPrincipalClass": "NSApplication",
    "InputMethodConnectionName": f"{bundle_id}_Connection",
    "InputMethodServerControllerClass": "ZooTypeInputController",
    "TISInputSourceID": bundle_id,
    "TISIntendedLanguage": "zh-Hant",
    "tsInputMethodCharacterRepertoireKey": ["Hant", "Latn"],
    "tsInputMethodIconFileKey": "menu.tiff",
    "tsInputMethodAlternateIconFileKey": "menu-dark.tiff",
    "tsInputMethodDefaultModeKey": mode_id,
    "ComponentInputModeDict": {
        "tsInputModeListKey": {
            mode_id: {
                "TISInputSourceID": mode_id,
                "TISIntendedLanguage": "zh-Hant",
                "tsInputModeMenuIconFileKey": "menu.tiff",
                "tsInputModeAlternateMenuIconFileKey": "menu-dark.tiff",
                "tsInputModePaletteIconFileKey": "menu.tiff",
                "tsInputModeScriptKey": "smTradChinese",
                "tsInputModeDefaultStateKey": True,
                "tsInputModeIsVisibleKey": True,
                "tsInputModePrimaryInScriptKey": True,
            }
        },
        "tsVisibleInputModeOrderedArrayKey": [mode_id],
    },
    "tsInputMethodScriptCodeKey": 2,
    "tsInputMethodPrimaryInScriptKey": True,
}
with (Path(app) / "Contents/Info.plist").open("wb") as output:
    plistlib.dump(metadata, output)
for locale in ["zh-Hant", "en"]:
    directory = Path(app) / "Contents/Resources" / f"{locale}.lproj"
    directory.mkdir(parents=True, exist_ok=True)
    # Explicit single-line names for the input-mode menu (not the application icon).
    lines = [f'"{key}" = "{name}";' for key in ["CFBundleName", "CFBundleDisplayName", mode_id]]
    (directory / "InfoPlist.strings").write_text("\n".join(lines) + "\n", encoding="utf-16")
