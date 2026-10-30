# References

Primary sources to check before implementing or changing behavior (per the dev charter: never invent APIs; cite the spec). Duplicates removed; notes mark entries that need care.

## Windows installation and deployment

- Windows 11 / 10 downloads: https://www.microsoft.com/software-download/windows11 · https://www.microsoft.com/software-download/windows10
- Support and docs: https://support.microsoft.com/windows · https://learn.microsoft.com/windows · https://learn.microsoft.com/windows/deployment
- Hardware / manufacturing: https://learn.microsoft.com/windows-hardware · https://learn.microsoft.com/windows-hardware/manufacture/desktop
- Windows Setup technical reference: https://learn.microsoft.com/windows-hardware/manufacture/desktop/windows-setup-technical-reference
- DISM: https://learn.microsoft.com/windows-hardware/manufacture/desktop/dism-overview · https://learn.microsoft.com/windows-hardware/manufacture/desktop/manage-wim-images · https://learn.microsoft.com/windows-hardware/manufacture/desktop/windows-image
- Setup automation (unattend): https://learn.microsoft.com/windows-hardware/manufacture/desktop/windows-setup-automation-overview
- Drivers: https://learn.microsoft.com/windows-hardware/drivers
- Win32: https://learn.microsoft.com/windows/win32 · https://learn.microsoft.com/windows/win32/api · https://learn.microsoft.com/windows/win32/fileio/file-systems · https://learn.microsoft.com/windows/win32/debug/pe-format · https://learn.microsoft.com/windows/win32/sysinfo/registry
- Security and cryptography: https://learn.microsoft.com/windows/security · https://learn.microsoft.com/windows/win32/seccrypto · https://learn.microsoft.com/windows/win32/seccrypto/cryptography-portal

## Disc-image and filesystem formats

- ECMA-119 (= ISO 9660): https://www.ecma-international.org/publications-and-standards/standards/ecma-119
- ISO 9660:1988: https://www.iso.org/standard/17505.html — withdrawn; current edition is ISO/IEC 9660:2023.
- https://www.iso.org/standard/17506.html is ISO 9661:1988 (withdrawn), not a CD/DVD filesystem standard — kept only because it was in the source list.
- UDF (OSTA): https://www.osta.org · https://www.osta.org/specs
- Media types: https://www.iana.org/assignments/media-types
- Linux kernel docs: https://www.kernel.org/doc/html/latest/filesystems/iso9660.html · https://www.kernel.org/doc/html/latest/filesystems/udf.html
- Tools: GRUB manual https://www.gnu.org/software/grub/manual · xorriso https://www.gnu.org/software/xorriso · cpio https://www.gnu.org/software/cpio · tar https://www.gnu.org/software/tar · libarchive https://www.libarchive.org · 7-Zip https://www.7-zip.org · libburnia https://www.libburnia-project.org
- cdrtools: listed as https://www.gnu.org/software/cdrtools, but cdrtools is not a GNU package; that URL is probably wrong. Unverified.
- Forensics / recovery: OSFMount https://www.osforensics.com/tools/mount-disk-images.html · The Sleuth Kit https://www.sleuthkit.org · Autopsy https://www.autopsy.com · CGSecurity (TestDisk/PhotoRec) https://www.cgsecurity.org

## Code and libraries

- Hosting: https://github.com · https://gitlab.com · https://codeberg.org · https://sourceforge.net
- Microsoft: https://learn.microsoft.com/dotnet · https://learn.microsoft.com/cpp · https://learn.microsoft.com/powershell · https://learn.microsoft.com/windows/wsl · https://learn.microsoft.com/windows/package-manager
- Win32 API headers: fileapi https://learn.microsoft.com/windows/win32/api/fileapi · winioctl https://learn.microsoft.com/windows/win32/api/winioctl · imagehlp https://learn.microsoft.com/windows/win32/api/imagehlp · wincrypt https://learn.microsoft.com/windows/win32/api/wincrypt · winnetwk https://learn.microsoft.com/windows/win32/api/winnetwk · winbase https://learn.microsoft.com/windows/win32/api/winbase · winreg https://learn.microsoft.com/windows/win32/api/winreg
- Frameworks: https://www.boost.org · https://www.qt.io · https://www.wxwidgets.org
- Rust: https://www.rust-lang.org · https://doc.rust-lang.org

## Boot, partitioning and virtual machines

- UEFI specifications: https://uefi.org/specifications · ATA (T13): https://www.t13.org
- Boot loaders: GRUB https://www.gnu.org/software/grub · Syslinux https://www.syslinux.org · rEFInd https://www.rodsbooks.com/refind
- Virtualization: QEMU https://www.qemu.org · VirtualBox https://www.virtualbox.org · VMware https://www.vmware.com · Hyper-V https://learn.microsoft.com/windows-server/virtualization/hyper-v · Evaluation Center https://www.microsoft.com/evalcenter
- Partitioning / FAT: GNU Parted https://www.gnu.org/software/parted · gdisk https://www.rodsbooks.com/gdisk · mtools https://www.gnu.org/software/mtools
- Installers / init: Anaconda https://www.fedoraproject.org/wiki/Anaconda · systemd https://systemd.io

## Validation, checksums and secure development

- NIST: https://csrc.nist.gov · https://www.nist.gov
- Crypto / signing: OpenSSL https://www.openssl.org · GnuPG https://www.gnupg.org · Sigstore https://www.sigstore.dev
- Code quality / security: SonarSource https://www.sonarsource.com · OWASP https://owasp.org · OWASP Top Ten https://owasp.org/www-project-top-ten (the list's `www.owasp.org` form is the old domain) · CWE https://cwe.mitre.org
- VirusTotal: https://www.virustotal.com
- Hashing: GNU coreutils https://www.gnu.org/software/coreutils/manual · Get-FileHash https://learn.microsoft.com/powershell/module/microsoft.powershell.utility/get-filehash
- RFCs: https://www.rfc-editor.org

## Testing and compatibility

- https://learn.microsoft.com/windows/apps · https://learn.microsoft.com/windows/win32/winprog/windows-api · Windows hardware test https://learn.microsoft.com/windows-hardware/test
- Wine https://www.winehq.org · ReactOS https://www.reactos.org
- Q&A: https://stackoverflow.com · https://serverfault.com · https://superuser.com
