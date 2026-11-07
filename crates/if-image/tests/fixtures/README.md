# Test fixtures

Made on Linux with independent tools so the readers are checked against
someone else's writer. Source tree: `readme.txt` ("hello world\n", mtime
2021-06-15 12:34:56 UTC), `empty.txt`, `Docs/big.bin` (5000 bytes, byte i =
(i*31+7) mod 256), `Docs/Deep/Deeper/leaf.dat` ("leaf"), `Empty Folder/`,
`Ünïcödé name – long file name.txt`.

| File | Command |
|---|---|
| `joliet_rr.iso` | `genisoimage -input-charset utf-8 -J -R -V "JOLIET RR"` (cdrkit 1.1.11) |
| `plain.iso` | `genisoimage -input-charset utf-8 -V PLAIN` |
| `udf.iso` | `genisoimage -input-charset utf-8 -udf -J -V "UDF VOL"` |
| `boot.iso` | `genisoimage -input-charset utf-8 -J -V BOOTME -b boot.img -no-emul-boot -boot-load-size 4 -eltorito-alt-boot -e efi.img -no-emul-boot` |
