use crate::config;
use crate::error::JanitorError;
use log::{debug, info, warn};
use std::collections::HashSet;
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

fn find_kernel_dir(module_dir: &Path) -> Result<PathBuf, JanitorError> {
    if !module_dir.exists() {
        return Err(JanitorError::NoKernelDir(module_dir.to_path_buf()));
    }
    let mut entries = fs::read_dir(module_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();

    // In the Live ISO there should be just one kernel installed
    entries.pop().ok_or_else(|| JanitorError::NoKernelDir(module_dir.to_path_buf()))
}

pub fn cleanup_drivers(
    config_paths: &[&str],
    module_dir: &Path,
    delete: bool,
) -> Result<(), JanitorError> {
    let (to_keep_re, to_delete_re) = config::read_config(config_paths)?;
    let kernel_dir = find_kernel_dir(module_dir)?;
    info!("Scanning kernel modules in {}", kernel_dir.display());

    let mut all_drivers = Vec::new();
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
            all_drivers.push(Driver::from_file(path)?);
        }
    }

    let mut to_keep: HashSet<Driver> = HashSet::new();
    let mut to_delete: HashSet<Driver> = HashSet::new();

    for driver in all_drivers {
        let kernel_path = driver.path.strip_prefix(&kernel_dir).unwrap().to_str()
            .ok_or_else(|| JanitorError::InvalidPath(driver.path.clone()))?;

        if to_delete_re.iter().any(|r| r.is_match(kernel_path)) {
            debug!("Marked for deletion by config: {}", driver.path.display());
            to_delete.insert(driver);
        } else if to_keep_re.iter().any(|r| r.is_match(kernel_path)) {
            debug!("Marked for keeping by config: {}", driver.path.display());
            to_keep.insert(driver);
        } else {
            debug!("Implicitly marked for deletion: {}", driver.path.display());
            to_delete.insert(driver);
        }
    }

    info!("Checking driver dependencies...");
    loop {
        let referenced: Vec<Driver> = to_delete
            .iter()
            .filter(|dd| to_keep.iter().any(|ad| ad.deps.contains(&dd.name)))
            .cloned()
            .collect();

        if referenced.is_empty() {
            break;
        }

        for d in &referenced {
            info!("Keep dependant driver {}", d.path.display());
            to_keep.insert(d.clone());
            to_delete.remove(d);
        }
    }

    let delete_drivers: Vec<_> = to_delete.iter().map(|d| &d.path).collect();
    info!("Found {} drivers to delete", delete_drivers.len());
    debug!("Drivers to delete: {:?}", delete_drivers);

    if delete {
        for driver_path in delete_drivers {
            info!("Deleting {}", driver_path.display());
            fs::remove_file(driver_path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        loop {
            let referenced: Vec<Driver> = to_delete
                .iter()
                .filter(|dd| to_keep.iter().any(|ad| ad.deps.contains(&dd.name)))
                .cloned()
                .collect();

            if referenced.is_empty() {
                break;
            }

            for d in &referenced {
                to_keep.insert(d.clone());
                to_delete.remove(d);
            }
        }

        assert!(to_keep.contains(&driver_a));
        assert!(to_keep.contains(&driver_b));
        assert!(to_keep.contains(&driver_c));
        assert!(!to_keep.contains(&driver_d));
        assert!(to_delete.contains(&driver_d));
        assert!(!to_delete.contains(&driver_a));
        assert!(!to_delete.contains(&driver_b));
        assert!(!to_delete.contains(&driver_c));
    }
}
