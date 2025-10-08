use crate::command::CommandRunner;
use crate::config;
use crate::error::JanitorError;
use crate::util;
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Driver {
    name: String,
    path: PathBuf,
    deps: Vec<String>,
}

impl Driver {
    fn from_file(path: &Path, runner: &dyn CommandRunner) -> Result<Self, JanitorError> {
        let deps_str = match runner.run("/usr/sbin/modinfo", &["-F", "depends", path.to_str().unwrap()]) {
            Ok(s) => s,
            Err(e) => {
                warn!("modinfo for {} failed: {}", path.display(), e);
                String::new()
            }
        };

        let deps = deps_str
            .trim()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

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
    runner: &dyn CommandRunner,
) -> Result<(), JanitorError> {
    let (to_keep_re, to_delete_re) = config::read_config(config_paths, runner)?;
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
            let driver = Driver::from_file(path, runner)?;
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
    use crate::command::CommandRunner;
    use std::collections::HashMap;
    use tempfile::tempdir;

    struct MockCommandRunner {
        responses: HashMap<String, String>,
    }

    impl CommandRunner for MockCommandRunner {
        fn run(&self, command: &str, args: &[&str]) -> Result<String, JanitorError> {
            let key = if args.is_empty() {
                command.to_string()
            } else {
                format!("{} {}", command, args.join(" "))
            };
            self.responses.get(&key).cloned().ok_or(JanitorError::Command(format!("Not mocked: {}", key)))
        }
    }

    #[test]
    fn test_cleanup_drivers() {
        let temp_dir = tempdir().unwrap();
        let module_dir = temp_dir.path();
        let kernel_dir = module_dir.join("6.1.0-test");
        fs::create_dir_all(&kernel_dir).unwrap();

        let mod_a_path = kernel_dir.join("a.ko");
        let mod_b_path = kernel_dir.join("b.ko");
        let mod_c_path = kernel_dir.join("c.ko");
        let mod_d_path = kernel_dir.join("d.ko");

        fs::write(&mod_a_path, "").unwrap();
        fs::write(&mod_b_path, "").unwrap();
        fs::write(&mod_c_path, "").unwrap();
        fs::write(&mod_d_path, "").unwrap();

        let config_path = temp_dir.path().join("test.conf");
        fs::write(&config_path, "a.ko").unwrap();

        let mut responses = HashMap::new();
        responses.insert(
            format!("/usr/sbin/modinfo -F depends {}", mod_a_path.display()),
            "b".to_string(),
        );
        responses.insert(
            format!("/usr/sbin/modinfo -F depends {}", mod_b_path.display()),
            "c".to_string(),
        );
        responses.insert(
            format!("/usr/sbin/modinfo -F depends {}", mod_c_path.display()),
            "".to_string(),
        );
        responses.insert(
            format!("/usr/sbin/modinfo -F depends {}", mod_d_path.display()),
            "".to_string(),
        );
        responses.insert("arch".to_string(), "x86_64".to_string());

        let runner = MockCommandRunner { responses };

        // Test dry run
        cleanup_drivers(&[config_path.to_str().unwrap()], module_dir, false, &runner).unwrap();
        assert!(mod_a_path.exists());
        assert!(mod_b_path.exists());
        assert!(mod_c_path.exists());
        assert!(mod_d_path.exists());

        // Test delete
        cleanup_drivers(&[config_path.to_str().unwrap()], module_dir, true, &runner).unwrap();
        assert!(mod_a_path.exists());
        assert!(mod_b_path.exists());
        assert!(mod_c_path.exists());
        assert!(!mod_d_path.exists());
    }
}
