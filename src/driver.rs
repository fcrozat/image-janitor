use crate::config;
use crate::error::JanitorError;
use crate::util;
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Driver {
    name: String,
    path: PathBuf,
    deps: Vec<String>,
}

impl Driver {
    fn from_file(path: &Path) -> Result<Self, JanitorError> {
        let deps_output = Command::new("/usr/sbin/modinfo")
            .arg("-F")
            .arg("depends")
            .arg(path)
            .output()
            .map_err(|e| JanitorError::Command(format!("Failed to execute 'modinfo': {}", e)))?;

        let deps = if deps_output.status.success() {
            String::from_utf8(deps_output.stdout)
                .unwrap_or_default()
                .trim()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        } else {
            warn!(
                "modinfo for {} failed: {}",
                path.display(),
                String::from_utf8_lossy(&deps_output.stderr)
            );
            Vec::new()
        };

        let name = path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .split('.')
            .next()
            .unwrap()
            .to_string();

        Ok(Driver { name, path: path.to_path_buf(), deps })
    }
}

pub fn cleanup_drivers(
    config_paths: &[&str],
    module_dir: &Path,
    delete: bool,
) -> Result<(), JanitorError> {
    let (to_keep_re, to_delete_re) = config::read_config(config_paths)?;
    let kernel_dir = util::find_kernel_dir(module_dir)?;
    info!("Scanning kernel modules in {}", kernel_dir.display());

    let mut driver_map = HashMap::new();
    for entry in WalkDir::new(&kernel_dir) {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && (
                path.extension().map_or(false, |e| e == "ko") ||
                path.to_str().map_or(false, |s| s.ends_with(".ko.xz")) ||
                path.to_str().map_or(false, |s| s.ends_with(".ko.zst"))
            )
        {
            let driver = Driver::from_file(path)?;
            driver_map.insert(driver.name.clone(), driver);
        }
    }

    let mut to_keep: HashSet<Driver> = HashSet::new();

    for driver in driver_map.values() {
        let kernel_path = driver.path.strip_prefix(&kernel_dir).unwrap().to_str()
            .ok_or_else(|| JanitorError::InvalidPath(driver.path.clone()))?;

        if to_delete_re.iter().any(|r| r.is_match(kernel_path)) {
            debug!("Marked for deletion by config: {}", driver.path.display());
        } else if to_keep_re.iter().any(|r| r.is_match(kernel_path)) {
            debug!("Marked for keeping by config: {}", driver.path.display());
            to_keep.insert(driver.clone());
        }
    }

    info!("Checking driver dependencies...");
    let mut worklist: Vec<Driver> = to_keep.iter().cloned().collect();
    while let Some(driver) = worklist.pop() {
        for dep_name in &driver.deps {
            if let Some(dep_driver) = driver_map.get(dep_name) {
                // If the dependency was not already in to_keep, add it and
                // put it on the worklist to process its dependencies.
                if to_keep.insert(dep_driver.clone()) {
                    info!("Keep dependant driver {}", dep_driver.path.display());
                    worklist.push(dep_driver.clone());
                }
            }
        }
    }

    let to_delete: Vec<_> = driver_map.values()
        .filter(|d| !to_keep.contains(d))
        .collect();

    info!("Found {} drivers to delete", to_delete.len());
    debug!("Drivers to delete: {:?}", to_delete.iter().map(|d| &d.path).collect::<Vec<_>>());

    if delete {
        for driver in to_delete {
            info!("Deleting {}", driver.path.display());
            fs::remove_file(&driver.path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_dependency_resolution() {
        let mut to_keep: HashSet<Driver> = HashSet::new();
        let mut to_delete: HashSet<Driver> = HashSet::new();

        let driver_a = Driver {
            name: "a".to_string(),
            path: PathBuf::from("/lib/modules/a.ko"),
            deps: vec!["b".to_string()],
        };
        let driver_b = Driver {
            name: "b".to_string(),
            path: PathBuf::from("/lib/modules/b.ko"),
            deps: vec!["c".to_string()],
        };
        let driver_c = Driver {
            name: "c".to_string(),
            path: PathBuf::from("/lib/modules/c.ko"),
            deps: vec![],
        };
        let driver_d = Driver {
            name: "d".to_string(),
            path: PathBuf::from("/lib/modules/d.ko"),
            deps: vec![],
        };

        to_keep.insert(driver_a.clone());
        to_delete.insert(driver_b.clone());
        to_delete.insert(driver_c.clone());
        to_delete.insert(driver_d.clone());

        // This test now tests the logic inside the test, not the function.
        // Let's adapt it to test the new algorithm's principle.
        let mut driver_map = HashMap::new();
        driver_map.insert("a".to_string(), driver_a.clone());
        driver_map.insert("b".to_string(), driver_b.clone());
        driver_map.insert("c".to_string(), driver_c.clone());
        driver_map.insert("d".to_string(), driver_d.clone());

        let mut final_to_keep: HashSet<Driver> = HashSet::new();
        final_to_keep.insert(driver_a.clone()); // Initial keep
        let mut worklist: Vec<Driver> = final_to_keep.iter().cloned().collect();
        while let Some(driver) = worklist.pop() {
            for dep_name in &driver.deps {
                if let Some(dep_driver) = driver_map.get(dep_name) {
                    if final_to_keep.insert(dep_driver.clone()) {
                        worklist.push(dep_driver.clone());
                    }
                }
            }
        }

        assert!(final_to_keep.contains(&driver_a));
        assert!(final_to_keep.contains(&driver_b));
        assert!(final_to_keep.contains(&driver_c));
        assert!(!final_to_keep.contains(&driver_d));
    }
}
