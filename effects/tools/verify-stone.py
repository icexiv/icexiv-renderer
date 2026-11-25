"""Acceptance against extracted Stone assets: verify command IDs, timing and both mounts.

python effects/tools/verify-stone.py EXTRACTED_ROOT stone.json effects/target/release/examples/timeline
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

if len(sys.argv) != 4:
    sys.exit(__doc__)
root, manifest, player = sys.argv[1:]
preview = Path(__file__).resolve().parent.parent / "examples/stone-preview.json"


def trace(manifest, scene=preview):
    proc = subprocess.run([player, root, manifest, scene, "70"], capture_output=True,
                          text=True, check=True)
    return [json.loads(line) for line in proc.stdout.splitlines()], proc.stderr


events = json.loads(Path(manifest).read_text())
assert [[(c["actor"], c["track"], c["command"]) for c in e["chain"]] for e in events] == [
    [(2, 4, 11), (2, 3, 5)], [(2, 4, 11), (2, 4, 6)], [(2, 6, 13)]
]
assert events[0]["avfx"] == events[1]["avfx"] == "vfx/common/eff/cmcn_shoot1c.avfx"
assert events[2]["avfx"] == "vfx/action/mgc_stone1/eff/mgc_ston1t0h.avfx"
assert [e["targets"][0]["kind"] for e in events] == [1, 2, 0]
frames, warnings = trace(manifest)
for tick, active in [(3, []), (4, [0, 1]), (6, [0, 1, 2]), (64, [2]), (66, [])]:
    assert frames[tick]["active"] == active, (tick, frames[tick])
assert frames[4]["quads"] > 0 and frames[4]["meshes"] > 0
assert frames[66]["quads"] == frames[66]["meshes"] == 0
assert "unsupported Item Binder" in warnings
assert "missing or invalid texture" not in warnings
resolved, warnings = trace(manifest, preview.with_name("stone-scene.json"))
assert [f["active"] for f in resolved] == [f["active"] for f in frames]
assert not any(w in warnings for w in ["staged playback:", "missing resolved Linear", "missing or invalid texture"])
with tempfile.TemporaryDirectory() as d:
    path = Path(d) / "event.json"
    for i, x in enumerate([-0.4, 0.4]):
        path.write_text(json.dumps([events[i]]))
        child, _ = trace(path)
        observed = child[4]["first_position"]
        assert max(abs(a-b) for a,b in zip(observed, [x, 0.9, 0])) < 1e-6, (i, observed)
print("Stone: three independently scheduled Documents; both ElementId 5 positions verified")
