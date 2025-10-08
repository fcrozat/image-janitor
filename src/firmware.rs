use crate::error::JanitorError;
use log::{debug, info};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

fn find_kernel_dir(module_dir: &Path) -> Result<PathBuf, JanitorError> {
    if !module_dir.exists() {
        return Err(JanitorError::NoKernelDir(module_dir.to_path_buf()));
    }
    let mut entries = fs::read_dir(module_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();

    entries.pop().ok_or_else(|| JanitorError::NoKernelDir(module_dir.to_path_buf()))
}

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
                    let pattern = if !fw_name.ends_with('*') {
                        format!("{{{},{}.xz,{}.zst}}", fw_name, fw_name, fw_name)
                    } else {
                        fw_name.to_string()
                    };

                    let full_pattern = fw_dir.join(pattern);
                    let matching = glob::glob(full_pattern.to_str().unwrap()).expect("Failed to read glob pattern");

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
    let mut result = vec![path.to_path_buf()];
    let mut current = path.to_path_buf();

    while fs::symlink_metadata(&current)?.file_type().is_symlink() {
        let target = fs::read_link(&current)?;
        let abs_target = if target.is_absolute() {
            target.clone()
        } else {
            current.parent().unwrap().join(&target)
        };

        let canonical_target = match fs::canonicalize(&abs_target) {
            Ok(p) => p,
            Err(_) => break, // Broken link
        };

        if result.contains(&canonical_target) || !canonical_target.starts_with(base_dir) {
            break; // Cycle or link outside base_dir
        }

        debug!(
            "Adding symlink {} -> {}",
            current.display(),
            target.display()
        );
        result.push(canonical_target.clone());
        current = canonical_target;
    }

    Ok(result)
}

pub fn cleanup_firmware(
    module_dir: &Path,
    fw_dir: &Path,
    delete: bool,
) -> Result<(), JanitorError> {
    let kernel_dir = find_kernel_dir(module_dir)?;
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
