import ziplinter

def main():
    info = ziplinter.parse_file("../testdata/unix.zip")
    assert info["comment"] == ""

    # created using `cat utf8-infozip.zip  time-infozip.zip >> concatenated.zip`
    with open("../testdata/concatenated.zip", "rb") as f:
        data = f.read()

    info1 = ziplinter.parse_bytes(data)

    start = 0
    for r in info1["parsed_ranges"]:
        if r["contains"] == "local file header":
            start = int(r["start"])
            break

    assert start == 162 

    info2 = ziplinter.parse_bytes(data[:start])

    assert info1 != info2

if __name__ == '__main__':
    main()
