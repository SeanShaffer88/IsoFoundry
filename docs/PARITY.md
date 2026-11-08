# Capability parity

Where IsoFoundry stands against the feature set of a complete
disc and USB toolkit, scored only from shipped, tested behaviour.
Have = works in the current build · Partial = works with a stated
limit · Missing = not there yet.

| Capability | State | Note |
|---|---|---|
| Read / extract ISO, BIN/CUE, CCD, CHD, NRG, MDS | Have | M1 |
| Create ISO/UDF/Joliet images | Have | M1 |
| Edit images | Have | M1 |
| Checksums (CRC32/MD5/SHA-1/SHA-256) + `.dat` lists | Have | M1 |
| Mount images | Have | M1 (Windows) |
| Bootable USB (files + raw modes) | Have | M2 |
| Windows installer USB | Have | M2 |
| Audio conversion + tags + ReplayGain | Partial | No Opus/Vorbis encoding (no permissive encoder) |
| Burn image / files / MP3 / video discs | Have | M4, simulated-drive gate |
| Multisession append | Have | M4 |
| Erase rewritable media | Have | M4 |
| Multi-drive burn (same job, several drives) | Have | RC, command line |
| Several jobs at once | Partial | Parallel task windows; no central queue |
| Disc → image (raw + subchannel) + dump reports | Have | M5, simulated-drive gate |
| Disc → disc | Partial | Via a temporary image, not on-the-fly |
| Secure audio rip + CUE + CD-Text read | Have | M5 |
| AccurateRip comparison | Partial | Checksums computed and logged; online comparison not wired |
| Burn audio CD with CD-Text and gaps | Partial | Full DAO + CD-Text on the simulated drive; real drives are track-at-once with fixed gaps |
| Surface / quality scan | Have | M5 |
| Recover files from a damaged disc | Have | M5 |
| Spanning backup / restore to discs | Have | M6 |
| USB stick → image window | Missing | Works from the command line (`usb read`); window planned after 1.0 |
| Blu-ray video (BDMV) authoring | Missing | Post-1.0 candidate |
| Copy-protection handling | Never | Out of scope by design |

Update rule: a row moves only when the behaviour is in a release
and covered by a test or a recorded field check.
