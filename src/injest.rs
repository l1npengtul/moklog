use crate::config::Config;
use crate::State;
use color_eyre::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::fs::create_dir;
use tokio::process::Command;

pub async fn pull_git(state: Arc<State>) -> Result<()> {
    if Path::new("sitecontent").is_dir() {
        Command::new("git").arg("pull").spawn()?.wait().await?;
    } else {
        Command::new("git")
            .arg("clone")
            .arg("-b")
            .arg(state.config.branch())
            .arg(state.config.git())
            .arg("sitecontent")
            .spawn()?
            .wait()
            .await?;
    }

    Ok(())
}

pub async fn
