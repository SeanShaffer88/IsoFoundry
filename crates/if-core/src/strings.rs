//! English UI strings (D-046: one table in code until a second language
//! ships; keys are the task and variant ids from [`crate::tasks`]).

/// Button caption, as in the approved mockup (D-045).
pub fn task_title(id: &str) -> &'static str {
    match id {
        "burn-image" => "Burn image file to disc",
        "burn-files" => "Burn or append files/folders to disc",
        "erase" => "Erase rewritable disc",
        "copy-to-image" => "Copy disc to image file",
        "copy-disc" => "Copy disc to another disc",
        "rip-audio" => "Rip Audio CD to mp3/flac/opus/wav...",
        "burn-audio" => "Burn Audio CD from mp3/m4a/flac/ape/wav...",
        "burn-mp3" => "Burn MP3 CD DVD",
        "convert-audio" => "Convert audio format...",
        "convert-image" => "Convert image file format",
        "create-image" => "Create image file from files/folders",
        "edit-image" => "Edit image file",
        "browse-image" => "Browse / Extract / Mount image file",
        "surface-test" => "Disc surface / quality test",
        "drive-info" => "View drive / disc information",
        "install-windows-usb" => "Install Windows to USB drive",
        "burn-video" => "Burn DVD / Blu-ray video folder",
        "backup" => "Spanning backup to discs",
        "checksum" => "Checksum / compare image",
        "recover-files" => "Recover files from damaged disc",
        "bootable-usb" => "Create bootable USB drive",
        _ => "",
    }
}

/// Tooltip: what the task does and its side effects (Directive V2.0 §1).
pub fn task_tip(id: &str) -> &'static str {
    match id {
        "burn-image" => "Write an ISO, BIN/CUE or other disc image to a blank or rewritable disc. Replaces nothing on your computer; the disc is written and optionally verified.",
        "burn-files" => "Build a data disc from files and folders, or add a new session to an appendable disc. Your files are only read.",
        "erase" => "Blank a CD-RW, DVD±RW or BD-RE. Everything on that disc is lost.",
        "copy-to-image" => "Read a disc into an image file on your computer. The disc is only read.",
        "copy-disc" => "Duplicate a disc onto another disc, through a temporary image or directly between two drives. Copy-protected discs are not supported.",
        "rip-audio" => "Read the tracks of an audio CD into music files. The CD is only read.",
        "burn-audio" => "Write music files to an audio CD that plays in CD players. Your files are only read.",
        "burn-mp3" => "Write music files as-is to a data CD or DVD for MP3-capable players.",
        "convert-audio" => "Convert music files to another format. Originals are kept; new files are written beside them or to a folder you pick.",
        "convert-image" => "Convert a disc image to another format (ISO, BIN/CUE, CCD, IMG, CHD). The original image is kept.",
        "create-image" => "Build an ISO or UDF image file from files and folders, optionally bootable. Your files are only read.",
        "edit-image" => "Add, remove or rename files in an image. A new image is saved; the original is kept.",
        "browse-image" => "Look inside an image, extract files from it, or mount it as a drive letter.",
        "surface-test" => "Read every sector of a disc to find unreadable areas; on supported drives also measure error rates. The disc is only read.",
        "drive-info" => "Show what your drives can do and what is on the inserted disc.",
        "install-windows-usb" => "Install a Windows that runs from a USB drive. Everything on the selected USB drive is erased. Asks for administrator rights.",
        "burn-video" => "Burn an existing DVD-Video (VIDEO_TS) or Blu-ray (BDMV) folder so players can read it. No video is converted.",
        "backup" => "Back up folders across as many discs as needed, optionally encrypted. Restorable with any ZIP tool.",
        "checksum" => "Compute CRC32, MD5, SHA-1 and SHA-256 of an image or disc, write a .dat hash file, or compare an image with a disc.",
        "recover-files" => "Copy readable files off a scratched or damaged disc, skipping bad areas.",
        "bootable-usb" => "Write a bootable image (for example a Windows or Linux installer) to a USB drive. Everything on the selected USB drive is erased. Asks for administrator rights.",
        _ => "",
    }
}

/// Split-button menu entry.
pub fn variant_label(id: &str) -> &'static str {
    match id {
        "burn-image-multi" => "Burn to several drives at once…",
        "burn-image-simulate" => "Simulate only (test write)…",
        "burn-files-multisession" => "Continue a multisession disc…",
        "burn-files-boot" => "Make a bootable disc (El Torito)…",
        "copy-to-image-raw" => "Raw copy with subchannel data…",
        "copy-disc-multi" => "Make several copies…",
        "copy-disc-on-the-fly" => "Copy on the fly (two drives)…",
        "backup-restore" => "Restore from a backup…",
        "usb-image-to-usb" => "Write image to USB drive…",
        "usb-usb-to-image" => "Save USB drive to image file…",
        _ => "",
    }
}

/// Group heading above the grid.
pub const SELECT_TASK: &str = "Please select a task:";

/// Message for a task whose milestone has not shipped.
pub fn not_yet(title: &str, milestone: &str, release: &str) -> String {
    format!(
        "\u{201C}{title}\u{201D} is planned for milestone {milestone} (version {release}).\n\nIt is not available in this build yet. See Help \u{2192} Roadmap for what each version brings."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks;

    #[test]
    fn every_task_and_variant_has_text() {
        for t in tasks::all() {
            assert!(!task_title(t.id).is_empty(), "title {}", t.id);
            assert!(!task_tip(t.id).is_empty(), "tip {}", t.id);
            for v in t.variants {
                assert!(!variant_label(v.id).is_empty(), "variant {}", v.id);
            }
        }
    }

    #[test]
    fn admin_tasks_say_so_in_their_tooltip() {
        for t in tasks::all().filter(|t| t.admin) {
            assert!(task_tip(t.id).contains("administrator"), "{}", t.id);
        }
    }

    #[test]
    fn destructive_tasks_warn_in_their_tooltip() {
        for id in ["erase", "install-windows-usb", "bootable-usb"] {
            assert!(task_tip(id).contains("erase") || task_tip(id).contains("lost"), "{id}");
        }
    }
}
