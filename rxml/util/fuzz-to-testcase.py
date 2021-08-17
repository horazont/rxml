#!/usr/bin/python3
import hashlib
import sys

for fn in sys.argv[1:]:
    with open(fn, "rb") as f:
        data = f.read()

    id_ = hashlib.sha256(data).hexdigest()[:16]

    def encode_rustbyte(v):
        if v == ord('"'):
            return '\\"'
        if v >= 0x20 and v < 0x7f:
            return chr(v)
        return "\\x{:02x}".format(v)

    data = "".join(map(encode_rustbyte, data))

    print("""
\t#[test]
\tfn fuzz_{id_}() {{
\t\tlet src = &b"{data}"[..];
\t\tlet result = run_fuzz_test(src);
\t\tassert!(result.is_err());
\t}}
""".format(id_=id_, data=data))
