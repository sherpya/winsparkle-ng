"""Inspect imports, exports and localized resource sizes without loading a PE DLL."""
import argparse
import json
from pathlib import Path
import struct

def inspect(path):
    data = Path(path).read_bytes()
    u16 = lambda offset: struct.unpack_from('<H', data, offset)[0]
    u32 = lambda offset: struct.unpack_from('<I', data, offset)[0]
    pe = u32(0x3c)
    if data[pe:pe+4] != b'PE\0\0': raise ValueError('Invalid PE signature')
    optional = pe+24
    directories = optional+(112 if u16(optional)==0x20b else 96)
    section_table = optional+u16(pe+20)
    sections=[]
    for index in range(u16(pe+6)):
        offset=section_table+index*40
        sections.append(dict(name=data[offset:offset+8].rstrip(b'\0').decode(),virtual_size=u32(offset+8),rva=u32(offset+12),raw_size=u32(offset+16),offset=u32(offset+20)))
    def locate(rva):
        for section in sections:
            delta=rva-section['rva']
            if 0<=delta<section['raw_size']: return section['offset']+delta
        raise ValueError(f'Unmapped RVA {rva:x}')
    def string(rva):
        offset=locate(rva)
        return data[offset:data.index(b'\0',offset)].decode('ascii')
    imports=[]
    if u32(directories+8):
        offset=locate(u32(directories+8))
        while any(data[offset:offset+20]):
            imports.append(string(u32(offset+12)));offset+=20
    exports=[]
    if u32(directories):
        offset=locate(u32(directories))
        names=locate(u32(offset+32))
        exports=[string(u32(names+i*4)) for i in range(u32(offset+24))]
    string_bytes=0;languages=set()
    if u32(directories+16):
        root=locate(u32(directories+16))
        def walk(relative,depth=0,kind=None):
            nonlocal string_bytes
            node=root+relative
            count=u16(node+12)+u16(node+14)
            for index in range(count):
                entry=node+16+index*8
                identity,target=u32(entry),u32(entry+4)
                item_kind=identity if depth==0 else kind
                if target&0x80000000: walk(target&0x7fffffff,depth+1,item_kind)
                elif item_kind==6:
                    string_bytes+=u32(root+target+4);languages.add(identity)
        walk(0)
    return dict(file=str(path),raw_bytes=len(data),machine=f'0x{u16(pe+4):04x}',imports=imports,exports=exports,export_count=len(exports),sections=sections,stringtable_bytes=string_bytes,stringtable_languages=[f'0x{v:04x}' for v in sorted(languages)])

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('dll',type=Path);parser.add_argument('--output',type=Path);args=parser.parse_args()
    report=json.dumps(inspect(args.dll),indent=2)
    if args.output: args.output.write_text(report+'\n',encoding='utf-8')
    else: print(report)
