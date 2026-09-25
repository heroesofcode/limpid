//! Filesystem facts that change what a byte count means.
//!
//! On a copy-on-write filesystem a deleted file frees nothing while a
//! snapshot still references its extents. Reporting "2.4 GiB reclaimable" on a
//! machine with a week of snapper snapshots is simply untrue, and it is the
//! single most common reason a cleaner is accused of doing nothing.

use std::path::{Path, PathBuf};

use crate::paths::Roots;

/// What the filesystem under a path implies for cleaning.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Volume {
    /// Mount point, as reported by the kernel.
    pub mount_point: PathBuf,
    /// Filesystem type, e.g. `btrfs`, `ext4`.
    pub filesystem: String,
    /// Whether the filesystem stores data compressed, making apparent size
    /// and on-disk size diverge.
    pub compressed: bool,
    /// Whether the filesystem shares extents between snapshots or reflinks.
    pub copy_on_write: bool,
}

impl Volume {
    /// Whether freed bytes may fail to show up in `df`.
    pub fn defers_reclaim(&self) -> bool {
        self.copy_on_write
    }
}

/// Read the mount table and find the entry governing `path`.
///
/// The longest matching mount point wins, which is how the kernel resolves it
/// too. Returns `None` when the mount table cannot be read, which is not
/// worth treating as an error — it only costs a caveat.
pub fn describe(path: &Path) -> Option<Volume> {
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    describe_from_mountinfo(&mounts, path)
}

/// Parse `mountinfo` content and find the entry governing `path`.
///
/// Split out from [`describe`] so the parsing can be tested against captured
/// tables rather than whatever the build machine happens to be running.
pub fn describe_from_mountinfo(mounts: &str, path: &Path) -> Option<Volume> {
    let mut best: Option<Volume> = None;

    for line in mounts.lines() {
        // Format: id parent major:minor root mount-point options... - fstype source super-options
        // The optional fields before the "-" separator are why this is not a
        // simple fixed-column split.
        let (before, after) = line.split_once(" - ")?;
        let mut head = before.split_whitespace();
        let mount_point = head.nth(4)?;
        let mut tail = after.split_whitespace();
        let filesystem = tail.next()?;
        let super_options = tail.nth(1).unwrap_or("");

        if !path.starts_with(mount_point) {
            continue;
        }
        if best
            .as_ref()
            .is_some_and(|found| found.mount_point.as_os_str().len() >= mount_point.len())
        {
            continue;
        }

        best = Some(Volume {
            mount_point: PathBuf::from(mount_point),
            filesystem: filesystem.to_owned(),
            compressed: super_options.contains("compress"),
            copy_on_write: matches!(filesystem, "btrfs" | "zfs" | "bcachefs"),
        });
    }

    best
}

/// Whether snapshots exist that could pin extents of files being removed.
///
/// Only the presence of snapshots matters here, not how much they hold:
/// counting that properly needs btrfs quota groups, which are off by default
/// and expensive to enable. Presence is enough to turn a promise into an
/// estimate.
pub fn has_snapshots(roots: &Roots) -> bool {
    let snapshots = roots.system("/.snapshots");
    let Ok(entries) = std::fs::read_dir(&snapshots) else {
        return false;
    };
    // snapper numbers its snapshots, so an empty or placeholder directory is
    // not evidence of anything.
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.parse::<u32>().is_ok())
    })
}

/// Caveats to attach to a scan of `roots`.
pub fn caveats(roots: &Roots) -> Vec<String> {
    let mut caveats = Vec::new();

    let volume = describe(&roots.home);

    if has_snapshots(roots) {
        caveats.push(
            "Snapshots exist on this system. Files they still reference keep their \
             space until the snapshot is removed, so freed space may not appear in \
             df straight away."
                .to_owned(),
        );
    } else if volume.as_ref().is_some_and(Volume::defers_reclaim) {
        caveats.push(
            "This is a copy-on-write filesystem. Space shared with a snapshot or a \
             reflinked copy is only freed once the last reference goes."
                .to_owned(),
        );
    }

    if volume.as_ref().is_some_and(|found| found.compressed) {
        caveats.push(
            "This filesystem stores data compressed. Limpid reports the space \
             actually occupied, which is smaller than the file sizes a file manager \
             shows."
                .to_owned(),
        );
    }

    caveats
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed copy of a real Omarchy mountinfo: btrfs subvolumes for /,
    // /home and the pacman cache, all on one compressed device.
    const MOUNTINFO: &str = "\
25 1 0:23 /@ / rw,relatime shared:1 - btrfs /dev/mapper/root rw,compress=zstd:3,ssd,subvol=/@
26 25 0:23 /@home /home rw,relatime shared:2 - btrfs /dev/mapper/root rw,compress=zstd:3,ssd,subvol=/@home
27 25 0:23 /@pkg /var/cache/pacman/pkg rw,relatime shared:3 - btrfs /dev/mapper/root rw,compress=zstd:3,subvol=/@pkg
28 25 0:6 / /dev rw,nosuid shared:4 - devtmpfs dev rw,size=4096k
29 25 259:1 / /boot rw,relatime shared:5 - vfat /dev/nvme0n1p1 rw,fmask=0137
";

    #[test]
    fn the_longest_matching_mount_point_wins() {
        let volume =
            describe_from_mountinfo(MOUNTINFO, Path::new("/var/cache/pacman/pkg/foo.zst")).unwrap();

        assert_eq!(volume.mount_point, Path::new("/var/cache/pacman/pkg"));
        assert_eq!(volume.filesystem, "btrfs");
    }

    #[test]
    fn a_path_under_no_specific_mount_falls_back_to_the_root() {
        let volume = describe_from_mountinfo(MOUNTINFO, Path::new("/usr/lib")).unwrap();

        assert_eq!(volume.mount_point, Path::new("/"));
    }

    #[test]
    fn btrfs_is_recognised_as_copy_on_write_and_compressed() {
        let volume = describe_from_mountinfo(MOUNTINFO, Path::new("/home/someone")).unwrap();

        assert!(volume.copy_on_write);
        assert!(volume.compressed);
        assert!(volume.defers_reclaim());
    }

    #[test]
    fn a_plain_filesystem_defers_nothing() {
        let volume = describe_from_mountinfo(MOUNTINFO, Path::new("/boot/initramfs")).unwrap();

        assert_eq!(volume.filesystem, "vfat");
        assert!(!volume.copy_on_write);
        assert!(!volume.compressed);
    }

    #[test]
    fn numbered_snapshot_directories_are_detected() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        std::fs::create_dir_all(roots.system("/.snapshots/1/snapshot")).unwrap();

        assert!(has_snapshots(&roots));
    }

    #[test]
    fn an_absent_or_unnumbered_snapshot_directory_is_not_evidence() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        assert!(!has_snapshots(&roots));

        std::fs::create_dir_all(roots.system("/.snapshots/README")).unwrap();
        assert!(!has_snapshots(&roots));
    }
}
