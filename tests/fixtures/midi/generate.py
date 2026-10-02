#!/usr/bin/env python3
"""Original synthetic SMF fixture; stdlib only, independent of the Rust codec."""
from pathlib import Path
import struct

def vlq(value):
    out=[value&127];value>>=7
    while value:out.insert(0,(value&127)|128);value>>=7
    return bytes(out)
def track(events,end):
    data=bytearray();previous=0
    for tick,rank,message in sorted(events,key=lambda e:(e[0],e[1])):
        data+=vlq(tick-previous)+bytes(message);previous=tick
    data+=vlq(end-previous)+b'\xff\x2f\0'
    return b'MTrk'+struct.pack('>I',len(data))+data
end=64*960
conductor=[(0,0,b'\xff\x51\x03\x07\xa1\x20'),(0,1,b'\xff\x58\x04\x04\x02\x18\x08'),
    (30720,0,b'\xff\x51\x03\x0a\x2c\x2b'),(30720,1,b'\xff\x58\x04\x07\x03\x18\x08')]
notes=[(0,0,[0xc0,17]),(0,1,[0xc2,24])]
for i in range(64):
    channel=0 if i%2==0 else 2;pitch=60+i%8;on=960*i+1
    notes.extend([(on,2,[0x90|channel,pitch,80+i%32]),(on+719,0,[0x80|channel,pitch,i%64])])
for i in range(9):notes.append((i*7680,1,[0xb0,1,i*15]))
original=Path(__file__).with_name('sixteen-bars-ppqn960.mid')
original.write_bytes(b'MThd'+struct.pack('>IHHH',6,1,2,960)+track(conductor,end)+track(notes,end))
