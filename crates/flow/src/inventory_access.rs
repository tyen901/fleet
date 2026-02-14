use crate::FlowConfig;
use inventory::RootInventory;
use std::path::Path;

pub(crate) fn open_inventory_root(
    cfg: &FlowConfig,
    inventory_db: &Path,
    profile_id: &str,
    dest_path: &Path,
) -> anyhow::Result<RootInventory> {
    let store = (cfg.inventory_store_factory)(inventory_db)?;
    let inventory = inventory::Inventory::from_store(store)?;
    inventory
        .open_root(profile_id, dest_path)
        .map_err(anyhow::Error::new)
}
