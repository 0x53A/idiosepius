"""Strict ETL7 M-type reader; raw data must remain local under target/.

Format: https://etlcdb.db.aist.go.jp/etlcdb/etln/form_m.htm
ETL Character Database / Electrotechnical Laboratory, Japanese Technical
Committee for Optical Character Recognition, ETL Character Database, 1973–1984.
The archive contains 33,600 records, although the overview states 16,800.
Keep repeated sheets and both sizes together by shared writer metadata.
"""

import hashlib
import unicodedata

RECORD_BYTES = 2052
SOURCES = {
    "ETL7LC_1": (
        9600,
        "348ab2afebecbce74012773aff3037a284ae6bfcce6755d5371e2b200fb22bf4",
    ),
    "ETL7LC_2": (
        7200,
        "1b7d76cfe1b72c65fbddf32f29a09701de7cbf6defe789fc04c6779622986480",
    ),
    "ETL7SC_1": (
        9600,
        "bd0a232f3047c2fdc7980c7b97f0d0353c53fb1472f337c3d930ad1a72c40851",
    ),
    "ETL7SC_2": (
        7200,
        "f8b5d57f022d93b6539fc6b470f474da64ca6eecd293621f05a28f07e9bdb8bc",
    ),
}
METHOD = "sex/age/industry/occupation/collection-date grouped across all files and sizes; conservative, writer identity unverified"


def decode(record):
    if len(record) != RECORD_BYTES:
        raise ValueError("ETL7 record must contain exactly 2052 bytes")
    try:
        character = unicodedata.normalize(
            "NFKC", bytes([record[6]]).decode("shift_jis")
        )
    except UnicodeDecodeError:
        character = ""
    label = (
        chr(ord(character) - 0x60)
        if len(character) == 1 and 0x30A1 <= ord(character) <= 0x30F6
        else None
    )
    metadata = (
        record[10],
        record[11],
        *(int.from_bytes(record[k : k + 2], "big") for k in (16, 18, 20)),
    )
    pixels = [value for byte in record[32:2048] for value in (byte >> 4, byte & 15)]
    return {
        "label": label,
        "sheet": int.from_bytes(record[4:6], "big"),
        "group": "etl7:" + ",".join(map(str, metadata)),
        "image": [pixels[y * 64 : (y + 1) * 64] for y in range(63)],
    }


def records(data_root):
    for filename, (count, checksum) in SOURCES.items():
        raw = (data_root / "ETL7" / filename).read_bytes()
        if (
            len(raw) != count * RECORD_BYTES
            or hashlib.sha256(raw).hexdigest() != checksum
        ):
            raise ValueError(f"Unexpected {filename} size or checksum")
        for index in range(count):
            row = decode(raw[index * RECORD_BYTES : (index + 1) * RECORD_BYTES])
            yield {"dataset": "etl7", "file": filename, "record": index, **row}


def self_test():
    raw = bytearray(RECORD_BYTES)
    raw[4:6] = (513).to_bytes(2, "big")
    raw[6] = 0xB1  # JIS X 0201 A -> hiragana a
    raw[10:12] = bytes([2, 37])
    raw[16:22] = bytes.fromhex("123456781e1a")
    raw[32] = 0xF1
    raw[2047] = 0x2E
    a = decode(raw)
    assert a["label"] == "あ" and a["sheet"] == 513
    assert a["group"] == "etl7:2,37,4660,22136,7706"
    assert len(a["image"]) == 63 and all(len(r) == 64 for r in a["image"])
    assert a["image"][0][:2] == [15, 1] and a["image"][-1][-2:] == [2, 14]
    raw[4:6] = (514).to_bytes(2, "big")
    raw[8] = 3  # scan quality and sheet do not define writer identity
    assert decode(raw)["group"] == a["group"]
    raw[11] = 38
    assert decode(raw)["group"] != a["group"]
    for code in (0xDE, 0xDF, 0xFF):
        raw[6] = code
        assert decode(raw)["label"] is None
    for bad in (raw[:-1], raw + b"x"):
        try:
            decode(bad)
        except ValueError:
            pass
        else:
            raise AssertionError("accepted malformed M-type record")
    print("ETL7 synthetic decoder tests passed")


if __name__ == "__main__":
    self_test()
