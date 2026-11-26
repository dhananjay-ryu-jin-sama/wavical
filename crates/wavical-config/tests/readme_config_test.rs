use std::fs;
use std::path::PathBuf;

/// Extract every fenced ```javascript block in README.md meant as a wavical.config.js config.
fn extract_readme_config_blocks(readme: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_js_fence = false;
    let mut current = String::new();

    for line in readme.lines() {
        let trimmed = line.trim();
        if !in_js_fence {
            if trimmed == "```javascript" || trimmed == "```js" {
                in_js_fence = true;
                current.clear();
            }
        } else if trimmed == "```" {
            in_js_fence = false;
            if current.contains("export default") || current.contains("module.exports") {
                blocks.push(current.clone());
            }
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }

    blocks
}

#[test]
fn readme_example_configs_validate_cleanly() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let readme_path = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("README.md");
    let readme = fs::read_to_string(&readme_path).expect("read README.md");

    let config_blocks = extract_readme_config_blocks(&readme);
    assert!(
        !config_blocks.is_empty(),
        "expected at least one ```javascript config block in README.md"
    );

    for (idx, block) in config_blocks.iter().enumerate() {
        let dir = tempfile::tempdir().expect("create tempdir");
        fs::write(dir.path().join("wavical.config.js"), block).expect("write wavical.config.js");
        let parsed = wavical_config::validate_config(dir.path()).unwrap_or_else(|err| {
            panic!(
                "README.md config block #{} failed validation: {err}\n---\n{block}",
                idx + 1
            )
        });
        assert!(
            !parsed.app_id.trim().is_empty(),
            "README.md config block #{} must have non-empty appId",
            idx + 1
        );
    }
}
