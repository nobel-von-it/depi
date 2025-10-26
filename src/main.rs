mod cargo;
mod commands;
mod dep;
mod storage;
mod utils;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    commands::handle_command().await?;
    Ok(())
}
