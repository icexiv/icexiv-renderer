"""Check selected extracted retail TMBs and the emitted scale for row 3800.

python effects/tools/verify-c012.py DATA_CHECKOUT EXTRACTED_ROOT TIMELINE_EXAMPLE
"""
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile


def payload(blob, command):
    p = 12
    for _ in range(struct.unpack_from("<I", blob, 8)[0]):
        tag, size = struct.unpack_from("<4sI", blob, p)
        if tag == b"C012" and struct.unpack_from("<H", blob, p + 8)[0] == command:
            assert size == 72
            return p + 8
        p += size
    raise AssertionError(command)


if len(sys.argv) != 4:
    sys.exit(__doc__)
checkout, root, player = sys.argv[1:]
tools = Path(__file__).resolve().parent
cases = [
    ("ability/2sw_dark/abl005", 12, "9e0bc616068794a7cf00cd6f97a2734782faecb2191d2b8a72b88adae92983e8", (1.25,)*3, (0,0,0), 0),
    ("mon_sp/d1017/mon_sp002", 10, "c372dfb9695e7f64563107c89e539d4f71d5b9c955366b32e2a95ab94603b01a", (1,1,1), (0,-1.7,1.25), 0),
    ("human_sp/c0101/human_sp069", 19, "78394a60584654eadc53d901114207c42e14fd5ce62c2b9c574d8df52709fe75", (1,1,1), (0,0,0), 2),
]
for key, cmd, digest, scale, pos, flags in cases:
    path = f"chara/action/{key}.tmb"
    raw = (Path(root) / path).read_bytes()
    assert hashlib.sha256(raw).hexdigest() == digest, path
    at = payload(raw, cmd)
    for field, expected in [(24,scale), (32,(0,0,0)), (40,pos), (48,(1,1,1,1))]:
        offset, count = struct.unpack_from("<II", raw, at+field)
        assert count == len(expected) and raw[at+offset:at+offset+4*count] == struct.pack(f"<{count}f", *expected)
    assert struct.unpack_from("<I", raw, at+56)[0] == flags

# The first row's first sampled quad actually uses the Document's scale. Compare
# with the exact same AVFX, bindings, clock and seed with only C012 scale changed.
path = f"chara/action/{cases[0][0]}.tmb"
events = json.loads(subprocess.check_output([sys.executable, tools/"import-action.py", checkout, root, path]))
event, = [e for e in events if e["chain"][-1]["command"] == 12]
assert event["avfx"] == "vfx/common/eff/cmab_shoot1c.avfx"
assert event["scale"] == [1.25]*3
with tempfile.TemporaryDirectory() as d:
    manifest = Path(d)/"cue.json"
    for scale, y in [(1.25,0.20), (1,0.16)]:
        manifest.write_text(json.dumps([{**event,"scale":[scale]*3}]))
        p = subprocess.run([player, root, manifest, tools.parent/"examples/stone-preview.json", "1"],
                           capture_output=True, text=True, check=True)
        f = json.loads(p.stdout)
        assert "missing or invalid texture" not in p.stderr
        assert f["active"] == [0] and f["quads"] == 1 and f["meshes"] == 3, f
        assert max(abs(a-b) for a,b in zip(f["first_position"], [0,y,0])) < 1e-6, f
print("Retail C012: scale, translation and flag 0x02 verified; authored scale changes measured geometry")
