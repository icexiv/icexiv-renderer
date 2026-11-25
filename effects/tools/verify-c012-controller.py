"""Offline C012 controller oracle. Usage: python verify-c012-controller.py ffxiv_dx11.exe

Unicorn 2.1.4 runs the selected original x64 code, with host object queries,
allocator, controller constructor/finalizer and handle readiness supplied by
the fixture. It neither starts the game nor edits the executable.
"""
from pathlib import Path
import hashlib
import itertools
import struct
import sys
from unicorn import Uc, UC_ARCH_X86, UC_MODE_64, UC_HOOK_CODE
from unicorn import x86_const as x

b = Path(sys.argv[1]).read_bytes()
assert hashlib.sha256(b).hexdigest() == '52042b60631cf2c48851b3aaee36e9ec4fedd6d7f2c68f0e9a8c3befb0af9f98'
m = Uc(UC_ARCH_X86, UC_MODE_64)
pe, = struct.unpack_from('<I', b, 60)
for i in range(struct.unpack_from('<H', b, pe + 6)[0]):
    size, rva, raw, off = struct.unpack_from('<4I', b, pe + 24 + struct.unpack_from('<H', b, pe + 20)[0] + 40*i + 8)
    addr = 0x140000000 + rva
    m.mem_map(addr, (max(size,raw)+4095)//4096*4096)
    m.mem_write(addr, b[off:off+raw])
m.mem_map(0x100000, 0x100000)
obj, payload, link, node, lookup, provider, state, vt, manager = range(0x100000, 0x109000, 0x1000)
stop, stack = 0x1ee000, 0x1f0008
def wr(a,n,size=8): m.mem_write(a,int(n).to_bytes(size,'little'))
def rd(a,size=8): return int.from_bytes(m.mem_read(a,size),'little')
def reg(name): return m.reg_read(getattr(x,'UC_X86_REG_'+name))
def put(name,v): m.reg_write(getattr(x,'UC_X86_REG_'+name),v)
def run(entry,**regs):
    for k,v in regs.items(): put(k,v)
    wr(stack,stop); put('RSP',stack)
    m.emu_start(entry,stop,count=50000)
    assert reg('RIP')==stop
calls=[]
answers={}
def stub(m,ip,size,_):
    calls.append((ip,reg('RCX'),reg('RDX'),reg('R8')))
    if ip in exits:
        put('RIP',stop);return
    put('RAX',answers.get(ip,0))
    sp=reg('RSP');put('RIP',rd(sp));put('RSP',sp+8)
exits={0x141913143,0x1419133f7,0x141912f2f,0x1419136d9}
hooks={0x14090da00,0x140634290,0x14089ae90,0x141c30940,0x14008fa70,0x141c2fce0,
       0x141c308c0,0x141c30710,0x141c2fd30,0x141d9d778,0x109000,0x109010,0x109020}
for ip in hooks|exits:m.hook_add(UC_HOOK_CODE,stub,begin=ip,end=ip)
wr(obj+0x48,payload);wr(obj+0x84,9,4);wr(obj+0x98,node)
wr(lookup,vt);wr(provider,vt);wr(vt+0xe8,0x109000);wr(vt+0x270,0x109010)
answers[0x140634290]=lookup
wr(0x142b03048,manager-8)
checks=0
for flags in range(8):
    wr(payload+0x38,flags,4)
    for eligible,code in itertools.product((0,1),(0,198,199)):
        calls.clear();answers[0x14090da00]=eligible;wr(link+0x178,code,4)
        run(0x141913117,RBX=obj,R14=link,R15=provider)
        assert (calls[-1][0]==0x141913143)==bool(flags&2 or eligible or code==198)
        assert any(c[0]==0x14090da00 for c in calls)==(flags&2==0)
        checks+=1
    for ordinal,table,cast,sub,bit in itertools.product((818,819),(0,1),(0,1),(0,1),(0,1)):
        answers.update({0x109000:ordinal,0x14089ae90:provider*cast,0x109010:state*sub})
        wr(0x1428bcdb0+32+8*ordinal,link*table)
        wr(state+0x3cc,bit,1);calls.clear()
        run(0x141912e9a,RBX=obj,R15=0)
        assert any(c==(0x141c30940,node,0,1) for c in calls)==bool(flags&4 and ordinal==818 and table and cast and sub and bit)
        checks+=1
    for available in (0,1):
        wr(manager,0);wr(manager+8,0);wr(obj+0xa0,0);calls.clear()
        answers[0x14008fa70]=node*available
        answers[0x141c2fce0]=node if flags&1 else obj+0xa8
        run(0x141913680,RCX=obj)
        assert rd(obj+0xa0)==(node if flags&1 and available else 0)
        assert rd(manager)==rd(manager+8)==rd(obj+0xa0)
        assert (calls[-1][0]==0x1419136d9)==bool(not flags&1 or available)
        checks+=1
        if flags&1 and available:
            wr(obj+0x98,node);calls.clear();run(0x1419138b0,RCX=obj)
            assert calls==[(0x141c30940,node,0,1)]
            assert rd(node+0xe8,1)==1 and not rd(obj+0xa0) and not rd(obj+0x98)
            wr(node+0x10,0x10,1)
            for ready in (0,1):
                answers[0x141c308c0]=ready;calls.clear()
                run(0x141c00a40,RCX=manager)
                assert rd(manager)==rd(manager+8)==(0 if ready else node)
                assert sum(c[0]==0x141d9d778 for c in calls)==ready
                checks+=1
            wr(obj+0x98,node)
    if not flags&1:
        # Embedded teardown walks +0x20, stops and virtually destructs each.
        next_node=node+0x100;wr(vt,0x109020);wr(node,vt);wr(next_node,vt)
        wr(node+0x20,next_node);wr(next_node+0x20,0);wr(obj+0x98,node)
        calls.clear();run(0x1419138b0,RCX=obj)
        assert [(c[0],c[1]) for c in calls]==[(fn,ptr) for ptr in (node,next_node) for fn in (0x141c30710,0x109020)]
        assert rd(obj+0x98)==0
        wr(obj+0x98,node);checks+=1
print(f'{checks} original branch/list boundaries passed')
