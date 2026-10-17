"""Reference implementation of docs/spec/hashing.md, used to generate the pinned test vectors.

Run with `python3 -I docs/spec/hashing_reference.py` (standard library only). It is the spec's
executable form; the Rust implementation must match it bit for bit.
"""


M=(1<<64)-1
G=0x9E3779B97F4A7C15
def mix(z):
    z=((z^(z>>30))*0xBF58476D1CE4E5B9)&M
    z=((z^(z>>27))*0x94D049BB133111EB)&M
    return z^(z>>31)
def absorb(h,v): return mix(((h^v)+G)&M)
def u(v): return v&M
def hash_(seed,level,x,y,z,salt):
    h=mix((seed+G)&M)
    for v in (level,u(x),u(y),u(z),salt): h=absorb(h,v)
    return h
class Rng:
    def __init__(s,st): s.s=st&M
    def next_u64(s):
        s.s=(s.s+G)&M; return mix(s.s)
    def next_u32(s): return s.next_u64()>>32
    def next_f32(s): return (s.next_u32()>>8)/16777216.0
    def below(s,n):
        assert n>0
        m=s.next_u64()*n; l=m&M
        if l<n:
            t=((1<<64)-n)%n
            while l<t: m=s.next_u64()*n; l=m&M
        return m>>64
    def range_i32(s,lo,hi): return lo+s.below(hi-lo)
    def chance(s,p): return s.next_f32()<p
    def pick(s,w):
        tot=sum(w)
        if tot==0: return None
        r=s.below(tot); c=0
        for i,x in enumerate(w):
            c+=x
            if r<c: return i
def fnv(b):
    h=0xcbf29ce484222325
    for c in b: h=((h^c)*0x100000001b3)&M
    return h
def seed_from_str(t):
    t=t.strip(" \t\n\r\x0b\x0c")
    if t=="": return "Err(Empty)"
    if t.isascii() and t.isdigit() and len(t)<=20 and int(t)<=M: return int(t)
    return fnv(t.encode())

if __name__ == "__main__":
    print("%016X" % hash_(42, 2, -1, 0, 6, 0x0301))
