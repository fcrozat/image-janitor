use crate::command::CommandRunner;
use crate::error::JanitorError;
use log::{debug, info};
use regex::Regex;
use std::fs;

/// Reads the configuration files and returns two lists of regexes: one for keeping and one for deleting.
pub fn read_config(
    paths: &[&str],
    runner: &dyn CommandRunner,
) -> Result<(Vec<Regex>, Vec<Regex>), JanitorError> {
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

    let arch = get_arch(runner)?;
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

fn get_arch(runner: &dyn CommandRunner) -> Result<String, JanitorError> {
    runner.run("arch", &[])
}

fn arch_filter(lines: Vec<String>, arch: &str) -> Vec<String> {
    let mut filtered = Vec::new();
    let mut skipping = false;
    let mut arch_tag: Option<String> = None;

    let start_tag_re = Regex::new(r"^\s*<(\w+)\s*>\s*$").unwrap();
    let end_tag_re = Regex::new(r"^\s*</\w+\s*>\s*$").unwrap();

    for line in lines {
        if let Some(captures) = start_tag_re.captures(&line) {
            let tag = captures.get(1).unwrap().as_str().to_string();
            skipping = tag != arch;
            arch_tag = Some(tag);
            continue;
        }

        if end_tag_re.is_match(&line) {
            skipping = false;
            arch_tag = None;
            continue;
        }

        if skipping {
            debug!(
                "Ignoring {} specific line: {}",
                arch_tag.as_deref().unwrap_or(""),
                line
            );
        } else {
            filtered.push(line);
        }
    }

    filtered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandRunner;
    use crate::error::JanitorError;
    use std::collections::HashMap;

    struct MockCommandRunner {
        commands: HashMap<String, String>,
    }

    impl CommandRunner for MockCommandRunner {
        fn run(&self, command: &str, _args: &[&str]) -> Result<String, JanitorError> {
            self.commands
                .get(command)
                .cloned()
                .ok_or_else(|| JanitorError::Command(format!("Command not found: {}", command)))
        }
    }

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

    #[test]
    fn test_read_config_with_arch() {
        let mut commands = HashMap::new();
        commands.insert("arch".to_string(), "x86_64".to_string());
        let runner = MockCommandRunner { commands };

        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("test.conf");
        fs::write(
            &config_path,
            "<x86_64>\n-delete_me\n</x86_64>\n<aarch64>\n-not_me\n</aarch64>\nkeep_me",
        )
        .unwrap();

        let (to_keep, to_delete) =
            read_config(&[config_path.to_str().unwrap()], &runner).unwrap();

        assert_eq!(to_keep.len(), 1);
        assert_eq!(to_delete.len(), 1);
        assert!(to_keep[0].is_match("keep_me"));
        assert!(to_delete[0].is_match("delete_me"));
    }
}
