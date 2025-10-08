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

fn remove_unused_files(
    fw_dir: &Path,
    required_fw: &HashSet<PathBuf>,
    delete: bool,
) -> Result<u64, JanitorError> {
    info!("Scanning for unused firmware files...");
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
                    debug!("Found unused firmware {}", path.display());
                }
            }
        }
    }
    Ok(unused_size)
}

fn remove_dangling_symlinks(fw_dir: &Path) -> Result<(), JanitorError> {
    info!("Removing dangling symlinks...");
    for entry in WalkDir::new(fw_dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_symlink() {
            // fs::metadata follows symlinks, so it will return an error for a dangling one.
            if fs::metadata(path).is_err() {
                info!("Deleting dangling symlink {}", path.display());
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

fn remove_empty_directories(fw_dir: &Path) -> Result<(), JanitorError> {
    info!("Removing empty directories...");
    // We need to walk from the deepest directories up to ensure parent directories become empty.
    let mut dirs_to_check: Vec<PathBuf> = WalkDir::new(fw_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.path().to_path_buf())
        .collect();

    // Sort by depth, deepest first.
    dirs_to_check.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

    for dir_path in dirs_to_check {
        // Only remove if it's empty and not the root firmware directory itself.
        if dir_path != fw_dir && fs::read_dir(&dir_path)?.next().is_none() {
            info!("Deleting empty directory {}", dir_path.display());
            fs::remove_dir(dir_path)?;
        }
    }
    Ok(())
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

    let unused_size = remove_unused_files(fw_dir, &required_fw, delete)?;

    if delete {
        remove_dangling_symlinks(fw_dir)?;
        remove_empty_directories(fw_dir)?;
    }

    info!("Potential savings: {} ({} MiB)", unused_size, unused_size >> 20);

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

    #[test]
    fn test_remove_unused_files() {
        let temp_dir = tempdir().unwrap();
        let fw_dir = temp_dir.path();

        let required_file_path = PathBuf::from("required.bin");
        let unused_file_path = PathBuf::from("unused.bin");

        fs::write(fw_dir.join(&required_file_path), "required_data").unwrap();
        fs::write(fw_dir.join(&unused_file_path), "unused_data").unwrap();

        let mut required_fw = HashSet::new();
        required_fw.insert(required_file_path.clone());

        // Test without deleting
        let unused_size = remove_unused_files(fw_dir, &required_fw, false).unwrap();
        assert_eq!(unused_size, 11); // "unused_data".len()
        assert!(fw_dir.join(&unused_file_path).exists());
        assert!(fw_dir.join(&required_file_path).exists());

        // Test with deleting
        let unused_size_del = remove_unused_files(fw_dir, &required_fw, true).unwrap();
        assert_eq!(unused_size_del, 11);
        assert!(!fw_dir.join(&unused_file_path).exists());
        assert!(fw_dir.join(&required_file_path).exists());
    }

    #[test]
    fn test_remove_dangling_symlinks() {
        let temp_dir = tempdir().unwrap();
        let fw_dir = temp_dir.path();

        let target_file = fw_dir.join("target.bin");
        let valid_symlink = fw_dir.join("valid_link");
        let dangling_symlink = fw_dir.join("dangling_link");

        fs::write(&target_file, "data").unwrap();
        symlink(&target_file, &valid_symlink).unwrap();
        symlink("non_existent_file", &dangling_symlink).unwrap();

        assert!(dangling_symlink.is_symlink());

        remove_dangling_symlinks(fw_dir).unwrap();

        assert!(valid_symlink.exists());
        assert!(!dangling_symlink.exists());
        assert!(!dangling_symlink.is_symlink()); // Should be completely gone
    }

    #[test]
    fn test_remove_empty_directories() {
        let temp_dir = tempdir().unwrap();
        let fw_dir = temp_dir.path();

        // Create a structure of directories
        let dir_a = fw_dir.join("a");
        let dir_b = dir_a.join("b"); // Will be empty
        let dir_c = dir_a.join("c");
        let dir_d = fw_dir.join("d"); // Will be empty

        fs::create_dir_all(&dir_b).unwrap();
        fs::create_dir_all(&dir_c).unwrap();
        fs::create_dir_all(&dir_d).unwrap();

        // Add a file to make dir_c non-empty
        fs::write(dir_c.join("file.txt"), "data").unwrap();

        assert!(dir_b.exists());
        assert!(dir_d.exists());

        remove_empty_directories(fw_dir).unwrap();

        // Assert empty directories are removed
        assert!(!dir_b.exists());
        assert!(!dir_d.exists());

        // Assert non-empty directories (and the root) remain
        assert!(dir_a.exists());
        assert!(dir_c.exists());
        assert!(fw_dir.exists());

        // Run again to ensure it handles the case where 'a' is now empty
        fs::remove_dir_all(&dir_c).unwrap();
        remove_empty_directories(fw_dir).unwrap();
        assert!(!dir_a.exists());
    }
}
