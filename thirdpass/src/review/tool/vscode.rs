use anyhow::{format_err, Context, Result};

/// Setup reviews directory within workspace.
pub fn setup_reviews_directory(
    workspace_directory: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let vscode_review_directory = workspace_directory.join(".vscode").join("reviews");
    std::fs::create_dir_all(&vscode_review_directory).context(format!(
        "Can't create directory: {}",
        vscode_review_directory.display()
    ))?;
    Ok(vscode_review_directory)
}

pub fn run(workspace_directory: &std::path::Path) -> Result<()> {
    let mut child = std::process::Command::new("code")
        .args(vec![
            "--wait",
            "--new-window",
            workspace_directory.to_str().ok_or(format_err!(
                "Failed to convert PathBuf to str: {}",
                workspace_directory.display()
            ))?,
        ])
        .current_dir(workspace_directory)
        .spawn()
        .expect("Failed to start vscode.");
    let _result = child.wait()?;
    Ok(())
}

pub fn setup() -> Result<()> {
    if !dialoguer::Confirm::new()
        .with_prompt(
            "This is the first time the review command has been executed.\n\
        Thirdpass will attempt to install the Thirdpass VSCode extension if it has not been installed.\n\
        Do you want to continue?",
        )
        .interact()?
    {
        return Err(format_err!("Abort VSCode Thirdpass extension installation."));
    }

    log::debug!("Attempting to install vscode extension.");
    let child = std::process::Command::new("code")
        .args(vec!["--install-extension", "thirdpass-org.thirdpass"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start vscode.");
    let output = child.wait_with_output()?;

    let stdout = std::str::from_utf8(&output.stdout)?;
    if stdout.contains("successfully installed") || stdout.contains("already installed") {
        log::debug!("Vscode extension already installed or installed successfully.");
        return Ok(());
    }

    Err(format_err!("Failed to install vscode thirdpass extension."))
}
