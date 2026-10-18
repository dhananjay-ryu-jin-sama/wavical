use std::path::Path;
use wavical_config::validate_config;

fn main() {
    let project_root = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    println!("Validating config in: {project_root}");
    match validate_config(Path::new(&project_root)) {
        Ok(config) => {
            println!("Config is valid! appId: {}", config.app_id);
            println!(
                "Window size: {}x{}",
                config.window_width(),
                config.window_height()
            );
            println!("Dev port: {}", config.dev_port());
            println!("Granted capabilities: {:?}", config.granted_capabilities());
        }
        Err(err) => {
            eprintln!("Validation error: {err}");
            if let Some(suggestion) = err.suggestion() {
                eprintln!("Did you mean: {suggestion}");
            }
        }
    }
}
