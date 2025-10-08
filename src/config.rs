use crate::error::JanitorError;
use log::{debug, info};
use regex::Regex;
use std::fs;
use std::process::Command;

/// Reads the configuration files and returns two lists of regexes: one for keeping and one for deleting.
pub fn read_config(paths: &[&str]) -> Result<(Vec<Regex>, Vec<Regex>), JanitorError> {
    let mut lines = Vec::<String>::new();
    for path in paths {
        info!("Reading config file: {}", path);
        let content = fs::read_to_string(path)
            .map_err(|e| JanitorError::ConfigRead(path.to_string(), e))?;
        lines.extend(content.lines().map(String::from));
    }

    let lines = lines
        .into_iter()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let arch = get_arch()?;
    debug!("Current architecture: {}", arch);
    let filtered_lines = arch_filter(lines, &arch);

    let (delete_lines, keep_lines): (Vec<_>, Vec<_>) = filtered_lines
        .into_iter()
        .partition(|l| l.starts_with('-'));

    let to_keep = keep_lines
        .into_iter()
        .map(|l| Regex::new(&l).map_err(JanitorError::Regex))
        .collect::<Result<Vec<_>, _>>()?;

    let to_delete = delete_lines
        .into_iter()
        .map(|l| Regex::new(l.strip_prefix('-').unwrap()).map_err(JanitorError::Regex))
        .collect::<Result<Vec<_>, _>>()?;

    Ok((to_keep, to_delete))
}

// This function calls an external command, which makes it harder to test.
// For a real-world scenario, this should be behind a trait that can be mocked.
fn get_arch() -> Result<String, JanitorError> {
    let output = Command::new("arch")
        .output()
        .map_err(|e| JanitorError::Command(format!("Failed to execute 'arch': {}", e)))?;

    if !output.status.success() {
        return Err(JanitorError::Command(
            "'arch' command failed".to_string(),
        ));
    }

    Ok(String::from_utf8(output.stdout).unwrap().trim().to_string())
}

fn arch_filter(lines: Vec<String>, arch: &str) -> Vec<String> {
    let mut filtered = Vec::new();
    let mut skipping = false;
    let mut arch_tag: Option<String> = None;

    for line in lines {
        if let Some(captures) = Regex::new(r"^\s*<\s*(\w+)\s*>\s*$").unwrap().captures(&line) {
            let tag = captures.get(1).unwrap().as_str().to_string();
            skipping = tag != arch;
            arch_tag = Some(tag);
            continue;
        }

        if Regex::new(r"^\s*</\s*\w+\s*>\s*$").unwrap().is_match(&line) {
            skipping = false;
            arch_tag = None;
            continue;
        }

        if skipping {
            debug!("Ignoring {} specific line: {}", arch_tag.as_deref().unwrap_or(""), line);
        } else {
            filtered.push(line);
        }
    }

    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arch_filter() {
        let lines = vec![
            "<x86_64>".to_string(),
            "intel_driver".to_string(),
            "</x86_64>".to_string(),
            "<aarch64>".to_string(),
            "arm_driver".to_string(),
            "</aarch64>".to_string(),
            "<ppc64le>".to_string(),
            "power_driver".to_string(),
            "</ppc64le>".to_string(),
            "<s390x>".to_string(),
            "ibm_driver".to_string(),
            "</s390x>".to_string(),
            "common_driver".to_string(),
        ];

        let x86_64_lines = arch_filter(lines.clone(), "x86_64");
        assert_eq!(x86_64_lines, vec!["intel_driver", "common_driver"]);

        let aarch64_lines = arch_filter(lines.clone(), "aarch64");
        assert_eq!(aarch64_lines, vec!["arm_driver", "common_driver"]);

        let ppc64le_lines = arch_filter(lines.clone(), "ppc64le");
        assert_eq!(ppc64le_lines, vec!["power_driver", "common_driver"]);

        let s390x_lines = arch_filter(lines.clone(), "s390x");
        assert_eq!(s390x_lines, vec!["ibm_driver", "common_driver"]);
    }
}
