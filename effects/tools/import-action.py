"""Run the TMB importer against an explicit icexiv-data checkout with tmb support.

python effects/tools/import-action.py DATA_CHECKOUT EXTRACTED_ROOT chara/action/key.tmb [--clips]
python effects/tools/import-action.py DATA_CHECKOUT --pack GAME/sqpack ACTION_ID CAS_DIR [--actor RACE] [logical PAP paths ...]
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

pack = len(sys.argv) >= 6 and sys.argv[2] == "--pack"
if not pack and len(sys.argv) != 4 and not (len(sys.argv) == 5 and sys.argv[4] == "--clips"):
    sys.exit(__doc__)
data, root, entry = sys.argv[1:4]
data = Path(data).resolve()
if not (data / "src/tmb.rs").is_file():
    sys.exit(f"{data} lacks the REA-supported tmb module")
with tempfile.TemporaryDirectory() as temp:
    manifest = Path(temp) / "Cargo.toml"
    source = Path(__file__).with_name("import_action.rs").resolve()
    manifest.write_text(f"""
[package]
name = "icexiv-import-action"
version = "0.1.0"
edition = "2024"
[dependencies]
icexiv-data = {{ path = {json.dumps(str(data))} }}
serde_json = "1"
tokio = {{version="1",features=["rt"],optional=true}}
sha2 = {{version="0.10",optional=true}}
[features]
data-snapshot = []
pack = ["icexiv-data/local","dep:tokio","dep:sha2"]
[[bin]]
name = "import-action"
path = {json.dumps(str(source))}
""")
    sys.exit(subprocess.run([
        "cargo", "run", "--quiet", "-j4", "--manifest-path", str(manifest),
        *(["--features", "pack"] if pack else []), "--", *sys.argv[2:]
    ]).returncode)
