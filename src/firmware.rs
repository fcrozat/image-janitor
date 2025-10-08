use crate::error::JanitorError;
use crate::util;
use log::{debug, info};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

fn get_required_firmware(kernel_dir: &Path, fw_dir: &Path) -> Result<HashSet<PathBuf>, JanitorError> {
    let mut required = HashSet::new();

    for entry in WalkDir::new(kernel_dir) {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && (
                path.extension().map_or(false, |e| e == "ko") ||
                path.to_str().map_or(false, |s| s.ends_with(".ko.xz")) ||
                path.to_str().map_or(false, |s| s.ends_with(".ko.zst"))
            )
        {
            let output = Command::new("/usr/sbin/modinfo")
                .arg("-F")
                .arg("firmware")
                .arg(path)
                .output()?;

            if output.status.success() {
                let firmware_list = String::from_utf8(output.stdout).unwrap_or_default();
                for fw_name in firmware_list.lines() {
                    let full_pattern_str = if !fw_name.ends_with('*') {
                        let base_path = fw_dir.join(fw_name);
                        format!("{}.{{,xz,zst}}", base_path.display())
                    } else {
                        fw_dir.join(fw_name).to_string_lossy().to_string()
                    };

                    let matching = glob::glob(&full_pattern_str).expect("Failed to read glob pattern");

                    for m in matching {
                        if let Ok(path) = m {
                            let symlinks = resolve_symlinks(&path, fw_dir)?;
                            required.extend(symlinks);
                        }
                    }
                }
            }
        }
    }

    Ok(required)
}

fn resolve_symlinks(path: &Path, base_dir: &Path) -> Result<Vec<PathBuf>, JanitorError> {
    let mut paths_to_keep = vec![path.to_path_buf()];

    // If the path is a symlink, we try to resolve it.
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        match fs::canonicalize(path) {
            Ok(final_target) => {
                // Only keep the target if it's within the firmware directory.
                // This prevents adding files from outside the scope (e.g., /usr/bin/true)
                // and also implicitly handles broken links, as canonicalize would fail.
                if final_target.starts_with(base_dir) {
                    debug!(
                        "Adding symlink target {} -> {}",
                        path.display(),
                        final_target.display()
                    );
                    paths_to_keep.push(final_target);
                }
            }
            Err(e) => debug!("Could not canonicalize symlink {}: {}", path.display(), e),
        }
    }

    Ok(paths_to_keep)
}

pub fn cleanup_firmware(
    module_dir: &Path,
    fw_dir: &Path,
    delete: bool,
) -> Result<(), JanitorError> {
    let kernel_dir = util::find_kernel_dir(module_dir)?;
    info!("Scanning kernel modules in {}", kernel_dir.display());

    let required_fw_abs = get_required_firmware(&kernel_dir, fw_dir)?;
    let required_fw: HashSet<_> = required_fw_abs.into_iter()
        .map(|p| p.strip_prefix(fw_dir).unwrap().to_path_buf())
        .collect();

    info!("Removing unused firmware...");
    let mut unused_size = 0;

    for entry in WalkDir::new(fw_dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_file() {
            let relative_path = path.strip_prefix(fw_dir).unwrap().to_path_buf();
            if !required_fw.contains(&relative_path) {
                unused_size += fs::metadata(path)?.len();
                if delete {
                    info!("Deleting unused firmware {}", path.display());
                    fs::remove_file(path)?;
                } else {
                    info!("Found unused firmware {}", path.display());
                }
            }
        }
    }

    if delete {
        info!("Removing dangling symlinks...");
        for entry in WalkDir::new(fw_dir).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_symlink() {
                // Check if the symlink is dangling
                if fs::metadata(path).is_err() {
                    info!("Deleting dangling symlink {}", path.display());
                    fs::remove_file(path)?;
                }
            }
        }

        // Removing empty directories.
        // We need to walk from the deepest directories up to ensure parent directories become empty.
        info!("Removing empty directories...");
        let mut dirs_to_check: Vec<PathBuf> = WalkDir::new(fw_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .map(|e| e.path().to_path_buf())
            .collect();

        dirs_to_check.sort_by_key(|p| p.components().count());
        dirs_to_check.reverse(); // Start from deepest

        for dir_path in dirs_to_check {
            // Only remove if it's empty and not the root firmware directory itself
            if dir_path != fw_dir && fs::read_dir(&dir_path)?.next().is_none() {
                info!("Deleting empty directory {}", dir_path.display());
                fs::remove_dir(dir_path)?;
            }
        }
    }

    info!("Unused firmware size: {} ({} MiB)", unused_size, unused_size >> 20);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn test_resolve_symlinks_single_file() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("file.bin");
        fs::write(&file_path, "data").unwrap();

        let resolved = resolve_symlinks(&file_path, temp_dir.path()).unwrap();
        assert_eq!(resolved, vec![file_path]);
    }

    #[test]
    fn test_resolve_symlinks_linear_chain() {
        let temp_dir = tempdir().unwrap();
        let base_dir = temp_dir.path();
        let file_path = base_dir.join("file.bin");
        let link1_path = base_dir.join("link1");
        let link2_path = base_dir.join("link2");

        fs::write(&file_path, "data").unwrap();
        symlink(&file_path, &link1_path).unwrap();
        symlink(&link1_path, &link2_path).unwrap();

        let resolved = resolve_symlinks(&link2_path, base_dir).unwrap();
        // The new implementation returns the starting link and the final canonical target.
        assert_eq!(resolved.len(), 2);
        assert!(resolved.contains(&file_path));
        assert!(resolved.contains(&link2_path));
    }

    #[test]
    fn test_resolve_symlinks_broken_link() {
        let temp_dir = tempdir().unwrap();
        let base_dir = temp_dir.path();
        let link_path = base_dir.join("link");

        symlink("non_existent_file", &link_path).unwrap();

        let resolved = resolve_symlinks(&link_path, base_dir).unwrap();
        // fs::canonicalize fails on broken links, so only the original path is returned.
        assert_eq!(resolved, vec![link_path]);
    }

    #[test]
    fn test_resolve_symlinks_cycle() {
        let temp_dir = tempdir().unwrap();
        let base_dir = temp_dir.path();
        let link1_path = base_dir.join("link1");
        let link2_path = base_dir.join("link2");

        symlink(&link2_path, &link1_path).unwrap();
        symlink(&link1_path, &link2_path).unwrap();

        let resolved = resolve_symlinks(&link1_path, base_dir).unwrap();
        // fs::canonicalize fails on link cycles, so only the original path is returned.
        assert_eq!(resolved.len(), 1);
        assert!(resolved.contains(&link1_path));
    }
}
