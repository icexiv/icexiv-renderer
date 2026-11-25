"""Offline Win64 TMTR differential oracle; requires unicorn 2.1.4.

Usage: python verify-tmtr-native.py ffxiv_dx11.exe file.tmb [file.tmb ...]
Print one JSON row per condition and input. Does not launch or attach to the game.
The original PE's expression builder, lexer, precedence parser, operators and
variable-bank reader run under x86 emulation. Only CRT services are shimmed.
"""
import hashlib
import json
from pathlib import Path
import re
import struct
import sys

from unicorn import Uc, UC_ARCH_X86, UC_MODE_64, UC_HOOK_CODE
from unicorn.x86_const import (
    UC_X86_REG_RSP, UC_X86_REG_RIP, UC_X86_REG_RAX, UC_X86_REG_RCX,
    UC_X86_REG_RDX, UC_X86_REG_R8, UC_X86_REG_R9,
)


def oracle(exe):
    assert hashlib.sha256(exe).hexdigest() == '52042b60631cf2c48851b3aaee36e9ec4fedd6d7f2c68f0e9a8c3befb0af9f98'
    m = Uc(UC_ARCH_X86, UC_MODE_64)
    # Load the preferred-image sections; no native imports, process or TLS.
    pe, = struct.unpack_from('<I', exe, 60)
    sections, = struct.unpack_from('<H', exe, pe+6)
    opt_size, = struct.unpack_from('<H', exe, pe+20)
    base, = struct.unpack_from('<Q', exe, pe+48)
    for i in range(sections):
        size, rva, raw, offset = struct.unpack_from('<IIII', exe, pe+24+opt_size+40*i+8)
        m.mem_map(base+rva, (max(size, raw)+4095)//4096*4096)
        m.mem_write(base+rva, exe[offset:offset+raw])
    arena, stop = 0x100000, 0x1ffff0
    m.mem_map(arena, 0x100000)
    reg = m.reg_read

    def string(addr):
        return bytes(m.mem_read(addr, 512)).split(b'\0')[0]

    def crt(m, ip, size, _):
        a, b, c, d = (reg(i) for i in [UC_X86_REG_RCX, UC_X86_REG_RDX, UC_X86_REG_R8, UC_X86_REG_R9])
        if ip == 0x14005fa20:  # signed %d / promoted %f
            fmt = string(c)
            s = (str(struct.unpack('<i', struct.pack('<I', d & 0xffffffff))[0]) if fmt == b'%d'
                 else format(struct.unpack('<d', struct.pack('<Q', d))[0], '.6f')).encode()
            assert fmt in (b'%d', b'%f') and len(s) < b
            m.mem_write(a, s+b'\0')
            result = len(s)
        elif ip == 0x141e0159c:  # ASCII tolower
            result = a + 32 if 65 <= a <= 90 else a
        elif ip == 0x141da5a30:  # memcpy token bytes
            m.mem_write(a, bytes(m.mem_read(b, c)))
            result = a
        elif ip == 0x1401e8a40:  # assign MSVC short string; tokenizer copies <=9
            assert c <= 9
            m.mem_write(a, bytes(m.mem_read(b, c))+b'\0')
            m.mem_write(a+16, struct.pack('<QQ', c, 15))
            result = a
        elif ip == 0x141dedd8c:  # per-thread errno address
            result = arena+0x10000
        elif ip == 0x141db9008:  # strtol decimal
            assert c == 10
            digits = re.match(rb'[0-9]+', string(a))[0]
            m.mem_write(b, struct.pack('<Q', a+len(digits)))
            result = int(digits)
        else:  # security cookie check
            assert ip == 0x141d9e070
            result = reg(UC_X86_REG_RAX)
        sp = reg(UC_X86_REG_RSP)
        m.reg_write(UC_X86_REG_RAX, result)
        m.reg_write(UC_X86_REG_RIP, int.from_bytes(m.mem_read(sp, 8), 'little'))
        m.reg_write(UC_X86_REG_RSP, sp+8)

    for addr in [0x14005fa20, 0x141e0159c, 0x141da5a30, 0x1401e8a40, 0x141dedd8c, 0x141db9008, 0x141d9e070]:
        m.hook_add(UC_HOOK_CODE, crt, begin=addr, end=addr)

    def call(ip, *args):
        for r, v in zip([UC_X86_REG_RCX, UC_X86_REG_RDX, UC_X86_REG_R8], args):
            m.reg_write(r, v)
        sp = arena+0xf0008
        m.mem_write(sp, struct.pack('<Q', stop))
        m.reg_write(UC_X86_REG_RSP, sp)
        m.emu_start(ip, stop, count=200000)
        assert reg(UC_X86_REG_RIP) == stop, hex(reg(UC_X86_REG_RIP))
        return reg(UC_X86_REG_RAX)

    def run(rows, variables):
        bank, block = arena, arena+0x800
        # This oracle accepts well-formed, bounded condition probes. Do not
        # feed unchecked counts to the native repeated-parenthesis builder.
        static = dict(enumerate(["+","-","*","/","%",">",">=","<","<=","~=","==","and","or","not"],3))
        static.update({20:"true",21:"false"})
        length = 0
        for op, word, bits in rows:
            signed = struct.unpack('<i',struct.pack('<I',word))[0]
            if op in (1,2): length += max(0, signed)
            elif op == 17: length += len(str(signed))
            elif op == 18: length += len(format(struct.unpack('<f',struct.pack('<I',bits))[0],'.6f'))
            elif op == 19: length += len(str(variables[word]))
            else: length += len(static[op])
        assert 0 < length < 384 and len(rows) <= 1024
        m.mem_write(bank, bytes(0x800))
        for key, value in variables.items():
            pool, index = key >> 28, key & 0xfffffff
            assert pool in (1, 2, 3) and index < (64 if pool == 1 else 32)
            m.mem_write(bank+{1:0,2:256,3:384}[pool]+4*index, struct.pack('<i', value))
        m.mem_write(block, struct.pack('<II', 8, len(rows))+b''.join(struct.pack('<III', *r) for r in rows))
        call(0x1418d1ef0, bank, block, 0)
        text = string(bank+0x200)
        assert len(text) < 384
        call(0x1418d2140, bank, arena+0x4000)
        return text.decode(), bool(m.mem_read(arena+0x4000, 1)[0])
    return run


def scripts(data):
    tag, size, count = struct.unpack_from('<4sII', data)
    assert tag == b'TMLB' and size == len(data)
    p = 12
    for _ in range(count):
        tag, length = struct.unpack_from('<4sI', data, p)
        s, p = p+8, p+length
        if tag != b'TMTR' or length != 24:
            continue
        rel, = struct.unpack_from('<I', data, s+12)
        if rel:
            offset, n = struct.unpack_from('<II', data, s+rel)
            assert offset == 8
            start = s+rel+offset
            rows = list(struct.iter_unpack('<III', data[start:start+n*12]))
            if rows:
                yield struct.unpack_from('<H',data,s)[0], rows


if __name__ == '__main__':
    run = oracle(Path(sys.argv[1]).read_bytes())
    for name in sys.argv[2:]:
        if name == '--operators':
            # Additional controlled inputs, separately identified from resources.
            for rows in (
                [(17,2,0),(14,0,0),(17,1,0)],
                [(17,1,0),(3,0,0),(17,2,0),(5,0,0),(17,3,0),(13,0,0),(17,7,0)],
                [(18,0,0xc0700000),(13,0,0),(17,0xfffffffd,0)],
                [(17,1234567890,0),(13,0,0),(17,123456789,0)],
            ):
                expr, admitted = run(rows, {})
                print(json.dumps(dict(script=rows, variables={}, expression=expr, admitted=admitted)))
            continue
        data = Path(name).read_bytes()
        for track, rows in scripts(data):
            keys = {r[1] for r in rows if r[0] == 19}
            for value in (0, 1, 35, 42):
                vars = {k:value for k in keys}
                expr, admitted = run(rows, vars)
                print(json.dumps(dict(tmb_sha256=hashlib.sha256(data).hexdigest(),track=track,
                    script=rows,variables=vars,expression=expr,admitted=admitted)))
