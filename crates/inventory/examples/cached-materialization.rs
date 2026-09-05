//! Measures the Fleet adapter against an explicitly pinned cached Swifty release.
use std::{path::Path, sync::Arc, time::Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 || !matches!(args[0].as_str(), "sync" | "verify") {
        return Err("expected sync|verify source-url repo-cache target inventory-db".into());
    }
    let setup = Instant::now();
    let input =
        fleet_flux::load_cached_swifty_materialization_input(&args[1], Path::new(&args[2]))?
            .ok_or("pinned cached input is missing")?;
    let revision = input
        .revision()
        .ok_or("cached input has no revision")?
        .to_owned();
    let inventory = Arc::new(fleet_inventory::FleetInventory::open(
        Path::new(&args[4]),
        Path::new(&args[3]),
        fleet_flux::swifty_profile_id(),
    )?);
    inventory.register_manifest(input.manifest())?;
    let setup_ns = setup.elapsed().as_nanos();
    let started = Instant::now();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let result = if args[0] == "verify" {
        let equal = fleet_flux::verify_manifest(
            Path::new(&args[3]),
            inventory,
            input,
            cancellation,
            None,
            None,
        )
        .await?;
        if !equal {
            return Err("target differs from pinned release".into());
        }
        serde_json::json!({"verified": true})
    } else {
        let outcome = fleet_flux::materialize(
            Path::new(&args[3]),
            inventory,
            input,
            cancellation,
            None,
            None,
        )
        .await?;
        serde_json::json!({"kept_files": outcome.kept_files,
            "reused_bytes": outcome.reused_bytes, "fetched_bytes": outcome.fetched_bytes,
            "written_bytes": outcome.written_bytes, "deleted_entries": outcome.deleted_entries,
            "peak_active_work": outcome.peak_active_work,
            "peak_buffer_bytes": outcome.peak_buffer_bytes,
            "peak_staging_bytes": outcome.peak_staging_bytes})
    };
    println!(
        "{}",
        serde_json::json!({"revision": revision, "setup_ns": setup_ns,
        "operation_ns": started.elapsed().as_nanos(), "result": result})
    );
    Ok(())
}
