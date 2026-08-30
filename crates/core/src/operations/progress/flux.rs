use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressScope,
    ProgressUnit,
};

pub(crate) struct FluxProgressObserver {
    publisher: OperationPublisher,
}

impl FluxProgressObserver {
    pub(crate) fn new(publisher: OperationPublisher) -> Self {
        Self { publisher }
    }
}

impl fleet_flux::ProgressObserver for FluxProgressObserver {
    fn update(&self, snapshot: fleet_flux::ProgressSnapshot) {
        let (stage, scope, text, unit) = match snapshot.phase {
            fleet_flux::MaterializationPhase::Verification => (
                OperationStage::VerifyingInventory,
                ProgressScope::InventoryVerify,
                "Verifying files",
                ProgressUnit::Files,
            ),
            fleet_flux::MaterializationPhase::Planning => (
                OperationStage::Sync,
                ProgressScope::MaterializationBytes,
                "Planning materialization",
                ProgressUnit::Paths,
            ),
            fleet_flux::MaterializationPhase::StoreDownload
            | fleet_flux::MaterializationPhase::ExternalReuse
            | fleet_flux::MaterializationPhase::LocalReuse
            | fleet_flux::MaterializationPhase::StageWrites => (
                OperationStage::Sync,
                ProgressScope::MaterializationBytes,
                "Materializing files",
                ProgressUnit::Bytes,
            ),
            fleet_flux::MaterializationPhase::FinalizeFiles => (
                OperationStage::Sync,
                ProgressScope::MaterializationFiles,
                "Finalizing files",
                ProgressUnit::Files,
            ),
            fleet_flux::MaterializationPhase::DeletePaths => (
                OperationStage::RemovingObsoleteFiles,
                ProgressScope::InventoryVerify,
                "Removing managed files",
                ProgressUnit::Files,
            ),
            fleet_flux::MaterializationPhase::Inventory => (
                OperationStage::Finalizing,
                ProgressScope::InventoryVerify,
                "Committing inventory",
                ProgressUnit::Paths,
            ),
            fleet_flux::MaterializationPhase::Complete
            | fleet_flux::MaterializationPhase::Failed => (
                OperationStage::Finalizing,
                ProgressScope::MaterializationBytes,
                "Finalizing",
                ProgressUnit::Paths,
            ),
        };
        self.publisher.progress(OperationProgressEvent {
            stage,
            scope,
            status_text: Some(text.to_string()),
            primary: ProgressMetric {
                label: None,
                done: Some(snapshot.completed),
                total: snapshot.total,
                unit,
            },
            secondary: None,
            throughput_bytes_per_sec: None,
            eta_seconds: None,
        });
    }
}
