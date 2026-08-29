//! Offline P3 measurement replay CLI. It has no live adapter or execution capability.

use std::{env, error::Error, fs, io, path::PathBuf};

use riftbot::{
    config::parse_toml,
    recording::{measurement::MeasurementReplayEngine, replay::ReplayEngine},
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_default();
    if command != "replay" {
        return Err(io::Error::other(
            "usage: p3-measurement replay <recording.jsonl> [config.toml] [output.json]",
        )
        .into());
    }
    let recording_path = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| io::Error::other("recording path is required"))?,
    );
    let config_path = arguments
        .next()
        .map_or_else(|| PathBuf::from("config/example.toml"), PathBuf::from);
    let output_path = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err(io::Error::other("too many arguments").into());
    }

    let config_text = fs::read_to_string(config_path)?;
    let config = parse_toml(&config_text)?;
    let replay =
        ReplayEngine::new(config.market_data.stale_after_ms)?.replay_file(&recording_path)?;
    let measurement = MeasurementReplayEngine::new(config)?.analyze(&replay)?;
    let json = serde_json::to_string_pretty(&measurement)?;
    if let Some(output_path) = output_path {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output_path)?;
        use std::io::Write;
        output.write_all(json.as_bytes())?;
        output.write_all(b"\n")?;
        output.sync_all()?;
    } else {
        println!("{json}");
    }
    Ok(())
}
