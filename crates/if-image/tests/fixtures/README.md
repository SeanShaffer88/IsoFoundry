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
| `plain_cd.chd` | `plain.iso` as MODE1/2352 BIN/CUE, then `chdman createcd -c cdzl` (MAME chdman 0.264). Overall SHA1 a2acc8b13372907b7ea10a9e14d577ce4594316b |
| `plain_dvd.chd` | `chdman createdvd -c zlib -i plain.iso` (chdman 0.264). Overall SHA1 9acf9b6d4fa58ee718e40717222d0f92728e57c0 |
